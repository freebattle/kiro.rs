//! POST /v1/responses 处理

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::{
    extract::State,
    http::StatusCode,
    response::{
        sse::{Event as SseEvent, KeepAlive, Sse},
        IntoResponse, Json, Response,
    },
};
use futures::stream::{self, StreamExt};
use serde_json::{json, Value};
use tokio::time::interval;
use uuid::Uuid;

use crate::anthropic::converter::{self, ConversionError};
use crate::anthropic::middleware::{AppState, CallerIdentity};
use crate::kiro::model::events::{Event, Event as KiroEvent};
use crate::kiro::model::requests::kiro::KiroRequest;
use crate::kiro::parser::decoder::EventStreamDecoder;
use crate::prompt_cache::PromptCacheTracker;
use crate::request_log::{RequestLogStore, RequestRecord};
use crate::token;

use super::converter::{
    collect_custom_tool_names, extract_custom_tool_call_input, responses_to_anthropic,
};
use super::history::expand_previous_response_history;
use super::input::parse_responses_input;
use super::store::{
    generate_output_item_id, generate_response_id, ResponseStore, StoreError,
};
use super::types::{
    OpenAIErrorResponse, ResponseContentPart, ResponseOutputItem, ResponsesObject,
    ResponsesRequest, ResponsesUsage,
};

const DEFAULT_RESPONSES_MODEL: &str = "claude-sonnet-4.5";
const PING_INTERVAL_SECS: u64 = 15;

/// 响应存储目录：data/responses（相对 cwd）
fn default_response_store() -> ResponseStore {
    ResponseStore::new("data/responses")
}

/// POST /v1/responses
pub async fn post_responses(
    State(state): State<AppState>,
    caller: Option<axum::Extension<CallerIdentity>>,
    axum::Json(mut payload): axum::Json<ResponsesRequest>,
) -> Response {
    let caller_name = caller.map(|c| c.0.name);
    let start_time = Instant::now();

    if payload.model.trim().is_empty() {
        payload.model = DEFAULT_RESPONSES_MODEL.to_string();
    }

    tracing::info!(
        model = %payload.model,
        stream = %payload.stream,
        has_prev = %payload.previous_response_id.is_some(),
        "Received POST /v1/responses request"
    );

    let provider = match &state.kiro_provider {
        Some(p) => p.clone(),
        None => {
            return openai_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "server_error",
                "Kiro API provider not configured",
            );
        }
    };

    let store = default_response_store();
    let store_response = payload.should_store();
    let stored_input = payload.input.clone();
    let resp_id = generate_response_id();

    // 展开 previous_response_id 历史
    let mut history_messages = Vec::new();
    if let Some(prev_id) = payload.previous_response_id.as_ref() {
        match store.load(prev_id) {
            Ok(prev) => {
                history_messages = expand_previous_response_history(&store, &prev);
            }
            Err(StoreError::NotFound) | Err(StoreError::Expired) => {
                return openai_error(
                    StatusCode::NOT_FOUND,
                    "invalid_request_error",
                    format!("previous_response_id not found: {prev_id}"),
                );
            }
            Err(e) => {
                return openai_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "server_error",
                    format!("failed to load previous_response_id: {e}"),
                );
            }
        }
    }

    let input_messages = match parse_responses_input(&payload.input) {
        Ok(m) => m,
        Err(e) => {
            return openai_error(StatusCode::BAD_REQUEST, "invalid_request_error", e.to_string());
        }
    };

    let mut final_messages = Vec::with_capacity(history_messages.len() + input_messages.len() + 1);
    final_messages.extend(history_messages);
    if let Some(instr) = payload.instructions.as_ref() {
        if !instr.trim().is_empty() {
            // 当前 turn 的 instructions 始终生效（放在历史之后）
            final_messages.push(super::types::OpenAIMessage::system_text(instr));
        }
    }
    final_messages.extend(input_messages);

    let anthropic_req = match responses_to_anthropic(&payload, &final_messages) {
        Ok(r) => r,
        Err(e) => {
            return openai_error(StatusCode::BAD_REQUEST, "invalid_request_error", e);
        }
    };

    let conversion_result = match converter::convert_request(&anthropic_req) {
        Ok(r) => r,
        Err(e) => {
            let (status, msg) = match &e {
                ConversionError::UnsupportedModel(m) => (
                    StatusCode::BAD_REQUEST,
                    format!("model not supported: {m}"),
                ),
                ConversionError::EmptyMessages => {
                    (StatusCode::BAD_REQUEST, "messages empty".to_string())
                }
            };
            return openai_error(status, "invalid_request_error", msg);
        }
    };

    let kiro_request = KiroRequest {
        conversation_state: conversion_result.conversation_state,
        profile_arn: None,
        agent_mode: conversion_result.agent_mode,
        additional_model_request_fields: conversion_result.additional_model_request_fields,
    };
    let request_body = match serde_json::to_string(&kiro_request) {
        Ok(b) => b,
        Err(e) => {
            return openai_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                format!("serialize request failed: {e}"),
            );
        }
    };

    if state.debug_logger.is_enabled() {
        let openai_body = serde_json::to_string_pretty(&payload).unwrap_or_default();
        state.debug_logger.log_request(
            &Uuid::new_v4().to_string()[..8],
            &openai_body,
            &request_body,
        );
    }

    let input_tokens = token::count_all_tokens(
        anthropic_req.model.clone(),
        anthropic_req.system.clone(),
        anthropic_req.messages.clone(),
        anthropic_req.tools.clone(),
    ) as i32;

    let session_fp = crate::prompt_cache::compute_session_fingerprint(
        None,
        anthropic_req.system.as_ref(),
        &anthropic_req.messages,
        &anthropic_req.model,
    );
    let cache_read_tokens = state
        .prompt_cache
        .compute_and_update(session_fp, input_tokens);

    let tool_name_map = conversion_result.tool_name_map;
    let custom_tool_names = collect_custom_tool_names(&payload);
    let model = payload.model.clone();

    if payload.stream {
        handle_stream(
            provider,
            request_body,
            model,
            input_tokens,
            cache_read_tokens,
            tool_name_map,
            custom_tool_names.clone(),
            state.request_log.clone(),
            state.prompt_cache.clone(),
            session_fp,
            start_time,
            caller_name,
            resp_id,
            payload,
            stored_input,
            store_response,
            store,
        )
        .await
    } else {
        handle_non_stream(
            provider,
            &request_body,
            &model,
            input_tokens,
            cache_read_tokens,
            tool_name_map,
            custom_tool_names,
            state.request_log.clone(),
            state.prompt_cache.clone(),
            session_fp,
            start_time,
            caller_name,
            resp_id,
            &payload,
            stored_input,
            store_response,
            &store,
        )
        .await
    }
}

async fn handle_non_stream(
    provider: Arc<crate::kiro::provider::KiroProvider>,
    request_body: &str,
    model: &str,
    input_tokens: i32,
    cache_read_tokens: i32,
    tool_name_map: HashMap<String, String>,
    custom_tool_names: std::collections::HashSet<String>,
    request_log: RequestLogStore,
    prompt_cache: Arc<PromptCacheTracker>,
    session_fp: u64,
    start_time: Instant,
    caller_name: Option<String>,
    resp_id: String,
    payload: &ResponsesRequest,
    stored_input: Value,
    store_response: bool,
    store: &ResponseStore,
) -> Response {
    let (response, credential_id) = match provider.call_api(request_body).await {
        Ok(r) => r,
        Err(e) => {
            log_failure(
                &request_log,
                model,
                input_tokens,
                cache_read_tokens,
                start_time,
                caller_name,
                false,
            );
            return map_provider_error(e);
        }
    };

    let body_bytes = match response.bytes().await {
        Ok(b) => b,
        Err(e) => {
            return openai_error(
                StatusCode::BAD_GATEWAY,
                "api_error",
                format!("read response failed: {e}"),
            );
        }
    };

    let collected = collect_kiro_events(&body_bytes, &tool_name_map, model, input_tokens);
    let output_tokens = estimate_output_tokens(&collected.text, &collected.tool_uses);

    let resp_obj = build_responses_object(
        &resp_id,
        model,
        &collected.text,
        &collected.tool_uses,
        collected.input_tokens.unwrap_or(input_tokens),
        output_tokens,
        payload,
        &custom_tool_names,
    );
    let mut resp_obj = resp_obj;
    resp_obj.stored_input = Some(stored_input);
    resp_obj.instructions = payload.instructions.clone();

    if store_response {
        if let Err(e) = store.save(&resp_obj) {
            tracing::warn!("[Responses] persist {} failed: {}", resp_obj.id, e);
        }
    }

    let duration_ms = start_time.elapsed().as_millis() as u64;
    let credits = collected.credits;
    let in_tok = collected.input_tokens.unwrap_or(input_tokens);
    // 固化本轮最终 input 为 prompt cache 基线（此前 OpenAI 路径漏写，导致 GPT 全量 cache）
    prompt_cache.update_actual_tokens(session_fp, in_tok);
    {
        let model_owned = model.to_string();
        let log = request_log.clone();
        tokio::spawn(async move {
            let safe_cache = clamp_cache_read(cache_read_tokens, in_tok);
            log.push(RequestRecord {
                model: model_owned,
                input_tokens: in_tok.max(0),
                output_tokens: output_tokens.max(0),
                cache_read_tokens: safe_cache,
                ttft_ms: None,
                duration_ms,
                timestamp: now_ms(),
                stream: false,
                credential_id: Some(credential_id),
                success: true,
                credits,
                caller: caller_name,
                thinking_effort: None,
            });
        });
    }

    // 对外不暴露 stored_input
    let mut public = resp_obj;
    public.stored_input = None;
    (StatusCode::OK, Json(public)).into_response()
}

struct CollectedOutput {
    text: String,
    tool_uses: Vec<CollectedToolUse>,
    input_tokens: Option<i32>,
    credits: f64,
}

struct CollectedToolUse {
    id: String,
    name: String,
    input: Value,
}

fn collect_kiro_events(
    body_bytes: &[u8],
    tool_name_map: &HashMap<String, String>,
    model: &str,
    estimated_input: i32,
) -> CollectedOutput {
    let mut decoder = EventStreamDecoder::new();
    let _ = decoder.feed(body_bytes);

    let mut text = String::new();
    let mut tool_uses = Vec::new();
    let mut tool_json_buffers: HashMap<String, String> = HashMap::new();
    let mut tool_names: HashMap<String, String> = HashMap::new();
    let mut context_input_tokens: Option<i32> = None;
    let mut credits = 0.0;

    for result in decoder.decode_iter() {
        let Ok(frame) = result else { continue };
        let Ok(event) = KiroEvent::from_frame(frame) else {
            continue;
        };
        match event {
            Event::AssistantResponse(resp) => {
                text.push_str(&resp.content);
            }
            Event::ToolUse(tool_use) => {
                tool_names
                    .entry(tool_use.tool_use_id.clone())
                    .or_insert_with(|| tool_use.name.clone());
                let buffer = tool_json_buffers
                    .entry(tool_use.tool_use_id.clone())
                    .or_default();
                buffer.push_str(&tool_use.input);
                if tool_use.stop {
                    let input: Value = if buffer.is_empty() {
                        json!({})
                    } else {
                        serde_json::from_str(buffer).unwrap_or_else(|_| json!({}))
                    };
                    let original_name = tool_name_map
                        .get(&tool_use.name)
                        .cloned()
                        .or_else(|| tool_names.get(&tool_use.tool_use_id).cloned())
                        .unwrap_or_else(|| tool_use.name.clone());
                    // 避免重复
                    if !tool_uses.iter().any(|t: &CollectedToolUse| t.id == tool_use.tool_use_id)
                    {
                        tool_uses.push(CollectedToolUse {
                            id: tool_use.tool_use_id,
                            name: original_name,
                            input,
                        });
                    }
                }
            }
            Event::ContextUsage(ctx) => {
                let window = crate::anthropic::converter::get_context_window_size(model);
                context_input_tokens =
                    Some((ctx.context_usage_percentage * (window as f64) / 100.0) as i32);
            }
            Event::Metering(m) => {
                credits += m.usage;
            }
            _ => {}
        }
    }

    // strip thinking tags for OpenAI surface
    let (clean_text, _) = strip_thinking_tags(&text);
    let _ = estimated_input;
    CollectedOutput {
        text: clean_text,
        tool_uses,
        input_tokens: context_input_tokens,
        credits,
    }
}

fn strip_thinking_tags(text: &str) -> (String, String) {
    // 简单剥离 <thinking>...</thinking>
    if let Some(start) = text.find("<thinking>") {
        if let Some(end_rel) = text[start..].find("</thinking>") {
            let end = start + end_rel + "</thinking>".len();
            let thinking = text[start + "<thinking>".len()..start + end_rel].to_string();
            let mut remaining = String::new();
            remaining.push_str(&text[..start]);
            remaining.push_str(&text[end..]);
            return (remaining.trim().to_string(), thinking);
        }
    }
    (text.to_string(), String::new())
}

fn estimate_output_tokens(text: &str, tools: &[CollectedToolUse]) -> i32 {
    let mut n = (text.len() as i32 / 4).max(1);
    for t in tools {
        n += 16;
        n += (t.name.len() as i32) / 4;
        n += (t.input.to_string().len() as i32) / 4;
    }
    n
}

fn clamp_cache_read(cache_read_tokens: i32, input_tokens: i32) -> i32 {
    cache_read_tokens.max(0).min(input_tokens.max(0))
}

fn build_responses_object(
    id: &str,
    model: &str,
    content: &str,
    tool_uses: &[CollectedToolUse],
    input_tokens: i32,
    output_tokens: i32,
    req: &ResponsesRequest,
    custom_tool_names: &std::collections::HashSet<String>,
) -> ResponsesObject {
    let mut output: Vec<ResponseOutputItem> = Vec::new();

    if !content.trim().is_empty() {
        output.push(ResponseOutputItem {
            id: generate_output_item_id("msg"),
            item_type: "message".to_string(),
            role: Some("assistant".to_string()),
            status: Some("completed".to_string()),
            content: vec![ResponseContentPart {
                part_type: "output_text".to_string(),
                text: Some(content.to_string()),
            }],
            call_id: None,
            name: None,
            arguments: None,
            input: None,
        });
    }

    for tu in tool_uses {
        if custom_tool_names.contains(&tu.name) {
            let input = extract_custom_tool_call_input(&tu.input.to_string());
            output.push(ResponseOutputItem {
                id: generate_output_item_id("ctc"),
                item_type: "custom_tool_call".to_string(),
                role: None,
                status: Some("completed".to_string()),
                content: vec![],
                call_id: Some(tu.id.clone()),
                name: Some(tu.name.clone()),
                arguments: None,
                input: Some(input),
            });
        } else {
            output.push(ResponseOutputItem {
                id: generate_output_item_id("fc"),
                item_type: "function_call".to_string(),
                role: None,
                status: Some("completed".to_string()),
                content: vec![],
                call_id: Some(tu.id.clone()),
                name: Some(tu.name.clone()),
                arguments: Some(tu.input.to_string()),
                input: None,
            });
        }
    }

    if output.is_empty() {
        output.push(ResponseOutputItem {
            id: generate_output_item_id("msg"),
            item_type: "message".to_string(),
            role: Some("assistant".to_string()),
            status: Some("completed".to_string()),
            content: vec![ResponseContentPart {
                part_type: "output_text".to_string(),
                text: Some(String::new()),
            }],
            call_id: None,
            name: None,
            arguments: None,
            input: None,
        });
    }

    ResponsesObject {
        id: id.to_string(),
        object: "response".to_string(),
        created_at: now_unix(),
        status: "completed".to_string(),
        model: model.to_string(),
        output,
        usage: ResponsesUsage {
            input_tokens,
            output_tokens,
            total_tokens: input_tokens + output_tokens,
        },
        previous_response_id: req.previous_response_id.clone(),
        metadata: req.metadata.clone(),
        error: None,
        instructions: None,
        stored_input: None,
        stored_at: Some(now_unix()),
    }
}

async fn handle_stream(
    provider: Arc<crate::kiro::provider::KiroProvider>,
    request_body: String,
    model: String,
    input_tokens: i32,
    cache_read_tokens: i32,
    tool_name_map: HashMap<String, String>,
    custom_tool_names: std::collections::HashSet<String>,
    request_log: RequestLogStore,
    prompt_cache: Arc<PromptCacheTracker>,
    session_fp: u64,
    start_time: Instant,
    caller_name: Option<String>,
    resp_id: String,
    payload: ResponsesRequest,
    stored_input: Value,
    store_response: bool,
    store: ResponseStore,
) -> Response {
    let (upstream, credential_id) = match provider.call_api_stream(&request_body).await {
        Ok(r) => r,
        Err(e) => {
            log_failure(
                &request_log,
                &model,
                input_tokens,
                cache_read_tokens,
                start_time,
                caller_name,
                true,
            );
            return map_provider_error(e);
        }
    };

    let created_at = now_unix();
    let initial = ResponsesObject {
        id: resp_id.clone(),
        object: "response".to_string(),
        created_at,
        status: "in_progress".to_string(),
        model: model.clone(),
        output: vec![],
        usage: ResponsesUsage::default(),
        previous_response_id: payload.previous_response_id.clone(),
        metadata: payload.metadata.clone(),
        error: None,
        instructions: None,
        stored_input: None,
        stored_at: None,
    };

    let body_stream = upstream.bytes_stream();
    let sse = stream::unfold(
        StreamState {
            body_stream: Box::pin(body_stream),
            decoder: EventStreamDecoder::new(),
            resp_id,
            model,
            created_at,
            initial,
            payload,
            stored_input,
            store_response,
            store,
            tool_name_map,
            custom_tool_names,
            input_tokens,
            cache_read_tokens,
            request_log,
            prompt_cache,
            session_fp,
            start_time,
            caller_name,
            credential_id,
            full_text: String::new(),
            tool_uses: Vec::new(),
            tool_json_buffers: HashMap::new(),
            tool_names: HashMap::new(),
            message_item_id: generate_output_item_id("msg"),
            message_started: false,
            output_index: 0,
            content_index: 0,
            finished: false,
            credits: 0.0,
            context_input_tokens: None,
            first_token_at: None,
            bootstrap_sent: 0,
            pending_events: VecDeque::new(),
            ping: interval(Duration::from_secs(PING_INTERVAL_SECS)),
        },
        |mut st| async move {
            // bootstrap: response.created + response.in_progress
            if st.bootstrap_sent == 0 {
                st.bootstrap_sent = 1;
                let ev = sse_json(
                    "response.created",
                    json!({"type":"response.created","response": st.initial}),
                );
                return Some((ev, st));
            }
            if st.bootstrap_sent == 1 {
                st.bootstrap_sent = 2;
                let ev = sse_json(
                    "response.in_progress",
                    json!({"type":"response.in_progress","response": st.initial}),
                );
                return Some((ev, st));
            }

            if let Some(ev) = st.pending_events.pop_front() {
                return Some((Ok(ev), st));
            }

            if st.finished {
                return None;
            }

            loop {
                tokio::select! {
                    biased;
                    chunk = st.body_stream.next() => {
                        match chunk {
                            Some(Ok(bytes)) => {
                                if let Err(e) = st.decoder.feed(&bytes) {
                                    tracing::warn!("responses decoder feed: {e}");
                                }
                                st.drain_decoder();
                                if let Some(ev) = st.pending_events.pop_front() {
                                    return Some((Ok(ev), st));
                                }
                            }
                            Some(Err(e)) => {
                                st.finished = true;
                                let ev = sse_json(
                                    "response.failed",
                                    json!({
                                        "type":"response.failed",
                                        "response":{
                                            "id": st.resp_id,
                                            "status":"failed",
                                            "error":{"type":"server_error","message": e.to_string()}
                                        }
                                    }),
                                );
                                return Some((ev, st));
                            }
                            None => {
                                // stream end
                                st.finish_stream();
                                st.finished = true;
                                if let Some(ev) = st.pending_events.pop_front() {
                                    return Some((Ok(ev), st));
                                }
                                return None;
                            }
                        }
                    }
                    _ = st.ping.tick() => {
                        return Some((Ok(SseEvent::default().comment("ping")), st));
                    }
                }
            }
        },
    );

    Sse::new(sse)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(PING_INTERVAL_SECS)))
        .into_response()
}

struct StreamState {
    body_stream: std::pin::Pin<
        Box<dyn futures::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>,
    >,
    decoder: EventStreamDecoder,
    resp_id: String,
    model: String,
    created_at: i64,
    initial: ResponsesObject,
    payload: ResponsesRequest,
    stored_input: Value,
    store_response: bool,
    store: ResponseStore,
    tool_name_map: HashMap<String, String>,
    custom_tool_names: std::collections::HashSet<String>,
    input_tokens: i32,
    cache_read_tokens: i32,
    request_log: RequestLogStore,
    prompt_cache: Arc<PromptCacheTracker>,
    session_fp: u64,
    start_time: Instant,
    caller_name: Option<String>,
    credential_id: u64,
    full_text: String,
    tool_uses: Vec<CollectedToolUse>,
    tool_json_buffers: HashMap<String, String>,
    tool_names: HashMap<String, String>,
    message_item_id: String,
    message_started: bool,
    output_index: usize,
    content_index: usize,
    finished: bool,
    credits: f64,
    context_input_tokens: Option<i32>,
    first_token_at: Option<Instant>,
    bootstrap_sent: u8,
    /// FIFO queue of SSE events ready to send
    pending_events: VecDeque<SseEvent>,
    ping: tokio::time::Interval,
}

impl StreamState {
    fn enqueue(&mut self, events: impl IntoIterator<Item = SseEvent>) {
        self.pending_events.extend(events);
    }

    fn drain_decoder(&mut self) {
        let mut frames = Vec::new();
        for result in self.decoder.decode_iter() {
            if let Ok(frame) = result {
                frames.push(frame);
            }
        }

        let mut events = Vec::new();
        for frame in frames {
            let Ok(event) = KiroEvent::from_frame(frame) else {
                continue;
            };
            match event {
                Event::AssistantResponse(resp) => {
                    if resp.content.is_empty() {
                        continue;
                    }
                    if self.first_token_at.is_none() {
                        self.first_token_at = Some(Instant::now());
                    }
                    self.full_text.push_str(&resp.content);
                    if !self.message_started {
                        self.message_started = true;
                        let idx = self.output_index;
                        let item_id = self.message_item_id.clone();
                        events.push(
                            SseEvent::default()
                                .event("response.output_item.added")
                                .data(
                                    json!({
                                        "type":"response.output_item.added",
                                        "output_index": idx,
                                        "item":{
                                            "id": item_id,
                                            "type":"message",
                                            "role":"assistant",
                                            "status":"in_progress",
                                            "content":[]
                                        }
                                    })
                                    .to_string(),
                                ),
                        );
                        events.push(
                            SseEvent::default()
                                .event("response.content_part.added")
                                .data(
                                    json!({
                                        "type":"response.content_part.added",
                                        "item_id": item_id,
                                        "output_index": idx,
                                        "content_index": self.content_index,
                                        "part":{"type":"output_text","text":""}
                                    })
                                    .to_string(),
                                ),
                        );
                    }
                    events.push(
                        SseEvent::default()
                            .event("response.output_text.delta")
                            .data(
                                json!({
                                    "type":"response.output_text.delta",
                                    "item_id": self.message_item_id,
                                    "output_index": self.output_index,
                                    "content_index": self.content_index,
                                    "delta": resp.content
                                })
                                .to_string(),
                            ),
                    );
                }
                Event::ToolUse(tool_use) => {
                    if self.first_token_at.is_none() {
                        self.first_token_at = Some(Instant::now());
                    }
                    self.tool_names
                        .entry(tool_use.tool_use_id.clone())
                        .or_insert_with(|| tool_use.name.clone());
                    let buffer = self
                        .tool_json_buffers
                        .entry(tool_use.tool_use_id.clone())
                        .or_default();
                    buffer.push_str(&tool_use.input);
                    if tool_use.stop {
                        if self.message_started {
                            let idx = self.output_index;
                            let item_id = self.message_item_id.clone();
                            let text = self.full_text.clone();
                            events.push(
                                SseEvent::default()
                                    .event("response.content_part.done")
                                    .data(
                                        json!({
                                            "type":"response.content_part.done",
                                            "item_id": item_id,
                                            "output_index": idx,
                                            "content_index": self.content_index,
                                            "part":{"type":"output_text","text": text}
                                        })
                                        .to_string(),
                                    ),
                            );
                            events.push(
                                SseEvent::default()
                                    .event("response.output_item.done")
                                    .data(
                                        json!({
                                            "type":"response.output_item.done",
                                            "output_index": idx,
                                            "item":{
                                                "id": item_id,
                                                "type":"message",
                                                "role":"assistant",
                                                "status":"completed",
                                                "content":[{"type":"output_text","text": text}]
                                            }
                                        })
                                        .to_string(),
                                    ),
                            );
                            self.message_started = false;
                            self.output_index += 1;
                        }
                        let input: Value = if buffer.is_empty() {
                            json!({})
                        } else {
                            serde_json::from_str(buffer).unwrap_or_else(|_| json!({}))
                        };
                        let original_name = self
                            .tool_name_map
                            .get(&tool_use.name)
                            .cloned()
                            .or_else(|| self.tool_names.get(&tool_use.tool_use_id).cloned())
                            .unwrap_or_else(|| tool_use.name.clone());
                        if !self.tool_uses.iter().any(|t| t.id == tool_use.tool_use_id) {
                            self.tool_uses.push(CollectedToolUse {
                                id: tool_use.tool_use_id.clone(),
                                name: original_name.clone(),
                                input: input.clone(),
                            });
                        }
                        let idx = self.output_index;
                        let args = input.to_string();
                        let is_custom = self.custom_tool_names.contains(&original_name);
                        if is_custom {
                            let ctc_id = generate_output_item_id("ctc");
                            let freeform = extract_custom_tool_call_input(&args);
                            events.push(
                                SseEvent::default()
                                    .event("response.output_item.added")
                                    .data(
                                        json!({
                                            "type":"response.output_item.added",
                                            "output_index": idx,
                                            "item":{
                                                "id": ctc_id,
                                                "type":"custom_tool_call",
                                                "status":"in_progress",
                                                "call_id": tool_use.tool_use_id,
                                                "name": original_name,
                                                "input":""
                                            }
                                        })
                                        .to_string(),
                                    ),
                            );
                            if !freeform.is_empty() {
                                events.push(
                                    SseEvent::default()
                                        .event("response.custom_tool_call_input.delta")
                                        .data(
                                            json!({
                                                "type":"response.custom_tool_call_input.delta",
                                                "item_id": ctc_id,
                                                "output_index": idx,
                                                "delta": freeform
                                            })
                                            .to_string(),
                                        ),
                                );
                            }
                            events.push(
                                SseEvent::default()
                                    .event("response.custom_tool_call_input.done")
                                    .data(
                                        json!({
                                            "type":"response.custom_tool_call_input.done",
                                            "item_id": ctc_id,
                                            "output_index": idx,
                                            "call_id": tool_use.tool_use_id,
                                            "name": original_name,
                                            "input": freeform
                                        })
                                        .to_string(),
                                    ),
                            );
                            events.push(
                                SseEvent::default()
                                    .event("response.output_item.done")
                                    .data(
                                        json!({
                                            "type":"response.output_item.done",
                                            "output_index": idx,
                                            "item":{
                                                "id": ctc_id,
                                                "type":"custom_tool_call",
                                                "status":"completed",
                                                "call_id": tool_use.tool_use_id,
                                                "name": original_name,
                                                "input": freeform
                                            }
                                        })
                                        .to_string(),
                                    ),
                            );
                        } else {
                            let fc_id = generate_output_item_id("fc");
                            events.push(
                                SseEvent::default()
                                    .event("response.output_item.added")
                                    .data(
                                        json!({
                                            "type":"response.output_item.added",
                                            "output_index": idx,
                                            "item":{
                                                "id": fc_id,
                                                "type":"function_call",
                                                "status":"in_progress",
                                                "call_id": tool_use.tool_use_id,
                                                "name": original_name,
                                                "arguments":""
                                            }
                                        })
                                        .to_string(),
                                    ),
                            );
                            events.push(
                                SseEvent::default()
                                    .event("response.function_call_arguments.delta")
                                    .data(
                                        json!({
                                            "type":"response.function_call_arguments.delta",
                                            "item_id": fc_id,
                                            "output_index": idx,
                                            "delta": args
                                        })
                                        .to_string(),
                                    ),
                            );
                            events.push(
                                SseEvent::default()
                                    .event("response.output_item.done")
                                    .data(
                                        json!({
                                            "type":"response.output_item.done",
                                            "output_index": idx,
                                            "item":{
                                                "id": fc_id,
                                                "type":"function_call",
                                                "status":"completed",
                                                "call_id": tool_use.tool_use_id,
                                                "name": original_name,
                                                "arguments": args
                                            }
                                        })
                                        .to_string(),
                                    ),
                            );
                        }
                        self.output_index += 1;
                    }
                }
                Event::ContextUsage(ctx) => {
                    let window =
                        crate::anthropic::converter::get_context_window_size(&self.model);
                    self.context_input_tokens =
                        Some((ctx.context_usage_percentage * (window as f64) / 100.0) as i32);
                }
                Event::Metering(m) => {
                    self.credits += m.usage;
                }
                _ => {}
            }
        }
        if !events.is_empty() {
            self.enqueue(events);
        }
    }


    fn finish_stream(&mut self) {
        // strip thinking tags from accumulated text for final object
        let (clean, _) = strip_thinking_tags(&self.full_text);
        self.full_text = clean;

        if self.message_started {
            let idx = self.output_index;
            let item_id = self.message_item_id.clone();
            let text = self.full_text.clone();
            self.enqueue([
                SseEvent::default()
                    .event("response.content_part.done")
                    .data(
                        json!({
                            "type":"response.content_part.done",
                            "item_id": item_id,
                            "output_index": idx,
                            "content_index": self.content_index,
                            "part":{"type":"output_text","text": text}
                        })
                        .to_string(),
                    ),
                SseEvent::default().event("response.output_item.done").data(
                    json!({
                        "type":"response.output_item.done",
                        "output_index": idx,
                        "item":{
                            "id": item_id,
                            "type":"message",
                            "role":"assistant",
                            "status":"completed",
                            "content":[{"type":"output_text","text": text}]
                        }
                    })
                    .to_string(),
                ),
            ]);
            self.message_started = false;
            self.output_index += 1;
        }

        let in_tok = self.context_input_tokens.unwrap_or(self.input_tokens);
        let out_tok = estimate_output_tokens(&self.full_text, &self.tool_uses);
        let mut resp_obj = build_responses_object(
            &self.resp_id,
            &self.model,
            &self.full_text,
            &self.tool_uses,
            in_tok,
            out_tok,
            &self.payload,
            &self.custom_tool_names,
        );
        resp_obj.created_at = self.created_at;
        resp_obj.stored_input = Some(self.stored_input.clone());
        resp_obj.instructions = self.payload.instructions.clone();

        if self.store_response {
            if let Err(e) = self.store.save(&resp_obj) {
                tracing::warn!("[Responses] persist {} failed: {}", resp_obj.id, e);
            }
        }

        let duration_ms = self.start_time.elapsed().as_millis() as u64;
        // 固化本轮最终 input 为 prompt cache 基线（此前 OpenAI 路径漏写，导致 GPT 全量 cache）
        self.prompt_cache
            .update_actual_tokens(self.session_fp, in_tok);
        {
            let log = self.request_log.clone();
            let model = self.model.clone();
            let cache_read_tokens = self.cache_read_tokens;
            let credits = self.credits;
            let credential_id = self.credential_id;
            let caller_name = self.caller_name.clone();
            let ttft_ms = self
                .first_token_at
                .map(|t| t.duration_since(self.start_time).as_millis() as u64);
            let safe_cache = clamp_cache_read(cache_read_tokens, in_tok);
            let in_tok_log = in_tok.max(0);
            let out_tok_log = out_tok.max(0);
            tokio::spawn(async move {
                log.push(RequestRecord {
                    model,
                    input_tokens: in_tok_log,
                    output_tokens: out_tok_log,
                    cache_read_tokens: safe_cache,
                    ttft_ms,
                    duration_ms,
                    timestamp: now_ms(),
                    stream: true,
                    credential_id: Some(credential_id),
                    success: true,
                    credits,
                    caller: caller_name,
                    thinking_effort: None,
                });
            });
        }

        let mut public = resp_obj;
        public.stored_input = None;
        self.enqueue(vec![
            SseEvent::default().event("response.completed").data(
                json!({"type":"response.completed","response": public}).to_string(),
            ),
            SseEvent::default().data("[DONE]"),
        ]);
    }
}

fn sse_json(event: &str, value: Value) -> Result<SseEvent, axum::Error> {
    Ok(SseEvent::default()
        .event(event)
        .data(value.to_string()))
}

fn openai_error(status: StatusCode, error_type: &str, message: impl Into<String>) -> Response {
    (status, Json(OpenAIErrorResponse::new(error_type, message))).into_response()
}

fn map_provider_error(e: anyhow::Error) -> Response {
    tracing::error!("Kiro provider error (responses): {e}");
    let msg = e.to_string();
    if msg.contains("401") || msg.to_lowercase().contains("unauthorized") {
        openai_error(StatusCode::UNAUTHORIZED, "authentication_error", msg)
    } else if msg.contains("429") {
        openai_error(StatusCode::TOO_MANY_REQUESTS, "rate_limit_error", msg)
    } else {
        openai_error(StatusCode::BAD_GATEWAY, "api_error", msg)
    }
}

fn log_failure(
    request_log: &RequestLogStore,
    model: &str,
    input_tokens: i32,
    cache_read_tokens: i32,
    start_time: Instant,
    caller_name: Option<String>,
    stream: bool,
) {
    let duration_ms = start_time.elapsed().as_millis() as u64;
    let model_owned = model.to_string();
    let log = request_log.clone();
    tokio::spawn(async move {
        log.push(RequestRecord {
            model: model_owned,
            input_tokens,
            output_tokens: 0,
            cache_read_tokens,
            ttft_ms: None,
            duration_ms,
            timestamp: now_ms(),
            stream,
            credential_id: None,
            success: false,
            credits: 0.0,
            caller: caller_name,
            thinking_effort: None,
        });
    });
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
