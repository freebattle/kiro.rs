//! Anthropic API Handler 函数

use std::convert::Infallible;

use crate::kiro::model::events::Event;
use crate::kiro::model::requests::kiro::KiroRequest;
use crate::kiro::parser::decoder::EventStreamDecoder;
use crate::request_log::{RequestLogStore, RequestRecord};
use crate::token;
use anyhow::Error;
use axum::{
    Json as JsonExtractor,
    body::Body,
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Json, Response},
};
use bytes::Bytes;
use futures::{Stream, StreamExt, stream};
use serde_json::json;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::time::interval;
use uuid::Uuid;

use super::converter::{ConversionError, claude_upstream_to_legacy_id, convert_request};
use super::middleware::{AppState, CallerIdentity};
use super::stream::{SseEvent, StreamContext};
use super::types::{
    CountTokensRequest, CountTokensResponse, ErrorResponse, MessagesRequest, Model, ModelsResponse,
};
use super::websearch;
use crate::prompt_cache;
use crate::prompt_cache::PromptCacheTracker;
use std::sync::Arc;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// 将 KiroProvider 错误映射为 HTTP 响应
fn map_provider_error(err: Error) -> Response {
    let err_str = err.to_string();

    // 上下文窗口满了（对话历史累积超出模型上下文窗口限制）
    if err_str.contains("CONTENT_LENGTH_EXCEEDS_THRESHOLD") {
        tracing::warn!(error = %err, "上游拒绝请求：上下文窗口已满（不应重试）");
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "invalid_request_error",
                "Context window is full. Reduce conversation history, system prompt, or tools.",
            )),
        )
            .into_response();
    }

    // 单次输入太长（请求体本身超出上游限制）
    if err_str.contains("Input is too long") {
        tracing::warn!(error = %err, "上游拒绝请求：输入过长（不应重试）");
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "invalid_request_error",
                "Input is too long. Reduce the size of your messages.",
            )),
        )
            .into_response();
    }
    tracing::error!("Kiro API 调用失败: {}", err);
    (
        StatusCode::BAD_GATEWAY,
        Json(ErrorResponse::new(
            "api_error",
            format!("上游 API 调用失败: {}", err),
        )),
    )
        .into_response()
}

/// GET /v1/models
///
/// 返回可用的模型列表（懒加载缓存 + TTL + 硬编码兜底）
pub async fn get_models(State(state): State<AppState>) -> impl IntoResponse {
    tracing::info!("Received GET /v1/models request");

    // 1. 尝试读缓存
    if let Some(remote_models) = state.models_cache.get().await {
        let models = filter_and_convert(&remote_models, state.include_open_source_models);
        return Json(ModelsResponse {
            object: "list".to_string(),
            data: models,
        });
    }

    // 2. 缓存未命中/过期，尝试远程拉取
    if let Some(provider) = &state.kiro_provider {
        match provider.fetch_available_models().await {
            Ok(resp) => {
                tracing::info!(
                    "ListAvailableModels 成功，获取到 {} 个模型",
                    resp.models.len()
                );
                state.models_cache.set(resp.models.clone()).await;
                let models = filter_and_convert(&resp.models, state.include_open_source_models);
                return Json(ModelsResponse {
                    object: "list".to_string(),
                    data: models,
                });
            }
            Err(e) => {
                tracing::warn!("ListAvailableModels 失败，使用兜底列表: {}", e);
            }
        }
    }

    // 3. 兜底：硬编码列表
    Json(ModelsResponse {
        object: "list".to_string(),
        data: fallback_models(),
    })
}

/// 判断模型 ID 是否为 Claude 系列（含 "auto" 通用模型）
fn is_claude_model(model_id: &str) -> bool {
    let id_lower = model_id.to_lowercase();
    id_lower.starts_with("claude") || id_lower == "auto"
}

/// 过滤并转换远程模型列表
fn filter_and_convert(
    remote: &[crate::kiro::model::available_models::RemoteModelInfo],
    include_open_source: bool,
) -> Vec<Model> {
    remote
        .iter()
        .filter(|m| include_open_source || is_claude_model(&m.model_id))
        .map(|m| {
            let max_tokens = m
                .token_limits
                .as_ref()
                .and_then(|t| t.max_output_tokens)
                .unwrap_or(64000) as i32;
            Model {
                id: claude_upstream_to_legacy_id(&m.model_id),
                object: "model".to_string(),
                created: 0,
                owned_by: "anthropic".to_string(),
                display_name: m.model_name.clone().unwrap_or_else(|| m.model_id.clone()),
                model_type: "chat".to_string(),
                max_tokens,
            }
        })
        .collect()
}

/// 硬编码兜底模型列表（远程不可用时返回）
fn fallback_models() -> Vec<Model> {
    vec![
        Model {
            id: "claude-opus-4-8".to_string(),
            object: "model".to_string(),
            created: 1780012800,
            owned_by: "anthropic".to_string(),
            display_name: "Claude Opus 4.8".to_string(),
            model_type: "chat".to_string(),
            max_tokens: 128000,
        },
        Model {
            id: "claude-opus-4-7".to_string(),
            object: "model".to_string(),
            created: 1778112000,
            owned_by: "anthropic".to_string(),
            display_name: "Claude Opus 4.7".to_string(),
            model_type: "chat".to_string(),
            max_tokens: 64000,
        },
        Model {
            id: "claude-opus-4-6".to_string(),
            object: "model".to_string(),
            created: 1770163200,
            owned_by: "anthropic".to_string(),
            display_name: "Claude Opus 4.6".to_string(),
            model_type: "chat".to_string(),
            max_tokens: 64000,
        },
        Model {
            id: "claude-sonnet-4-6".to_string(),
            object: "model".to_string(),
            created: 1771286400,
            owned_by: "anthropic".to_string(),
            display_name: "Claude Sonnet 4.6".to_string(),
            model_type: "chat".to_string(),
            max_tokens: 64000,
        },
        Model {
            id: "claude-sonnet-4-5".to_string(),
            object: "model".to_string(),
            created: 1759104000,
            owned_by: "anthropic".to_string(),
            display_name: "Claude Sonnet 4.5".to_string(),
            model_type: "chat".to_string(),
            max_tokens: 64000,
        },
        Model {
            id: "claude-haiku-4-5".to_string(),
            object: "model".to_string(),
            created: 1760486400,
            owned_by: "anthropic".to_string(),
            display_name: "Claude Haiku 4.5".to_string(),
            model_type: "chat".to_string(),
            max_tokens: 64000,
        },
    ]
}

/// POST /v1/messages
///
/// 创建消息（对话）
pub async fn post_messages(
    State(state): State<AppState>,
    caller: Option<axum::Extension<CallerIdentity>>,
    JsonExtractor(payload): JsonExtractor<MessagesRequest>,
) -> Response {
    let caller_name = caller.map(|c| c.0.name);
    let start_time = Instant::now();
    tracing::info!(
        model = %payload.model,
        max_tokens = %payload.max_tokens,
        stream = %payload.stream,
        message_count = %payload.messages.len(),
        "Received POST /v1/messages request"
    );
    // 检查 KiroProvider 是否可用
    let provider = match &state.kiro_provider {
        Some(p) => p.clone(),
        None => {
            tracing::error!("KiroProvider 未配置");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse::new(
                    "service_unavailable",
                    "Kiro API provider not configured",
                )),
            )
                .into_response();
        }
    };

    // 检查是否为 WebSearch 请求
    if websearch::has_web_search_tool(&payload) {
        tracing::info!("检测到 WebSearch 工具，路由到 WebSearch 处理");

        // 估算输入 tokens
        let input_tokens = token::count_all_tokens(
            payload.model.clone(),
            payload.system.clone(),
            payload.messages.clone(),
            payload.tools.clone(),
        ) as i32;

        return websearch::handle_websearch_request(provider, &payload, input_tokens).await;
    }

    // 转换请求
    let conversion_result = match convert_request(&payload) {
        Ok(result) => result,
        Err(e) => {
            let (error_type, message) = match &e {
                ConversionError::UnsupportedModel(model) => {
                    ("invalid_request_error", format!("模型不支持: {}", model))
                }
                ConversionError::EmptyMessages => {
                    ("invalid_request_error", "消息列表为空".to_string())
                }
            };
            tracing::warn!("请求转换失败: {}", e);
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse::new(error_type, message)),
            )
                .into_response();
        }
    };

    // 构建 Kiro 请求（profile_arn 由 provider 层根据实际凭据注入）
    let kiro_request = KiroRequest {
        conversation_state: conversion_result.conversation_state,
        profile_arn: None,
    };

    let request_body = match serde_json::to_string(&kiro_request) {
        Ok(body) => body,
        Err(e) => {
            tracing::error!("序列化请求失败: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "internal_error",
                    format!("序列化请求失败: {}", e),
                )),
            )
                .into_response();
        }
    };

    tracing::debug!("Kiro request body: {}", request_body);

    // 调试日志：记录原始请求和转换后的请求
    if state.debug_logger.is_enabled() {
        let anthropic_body = serde_json::to_string_pretty(&payload).unwrap_or_default();
        state.debug_logger.log_request(
            &Uuid::new_v4().to_string()[..8],
            &anthropic_body,
            &request_body,
        );
    }

    // 计算 prompt cache 模拟（必须在 system/messages 被 move 之前）
    let session_fp = prompt_cache::compute_session_fingerprint(
        payload.metadata.as_ref().and_then(|m| m.user_id.as_deref()),
        payload.system.as_ref(),
        &payload.messages,
        &payload.model,
    );

    // 估算输入 tokens
    let input_tokens = token::count_all_tokens(
        payload.model.clone(),
        payload.system,
        payload.messages,
        payload.tools,
    ) as i32;

    let cache_read_tokens = state
        .prompt_cache
        .compute_and_update(session_fp, input_tokens);

    // 检查是否启用了thinking
    let thinking_enabled = payload
        .thinking
        .as_ref()
        .map(|t| t.is_enabled())
        .unwrap_or(false);

    let thinking_effort = payload.output_config.as_ref().map(|c| c.effort.clone());

    let tool_name_map = conversion_result.tool_name_map;

    if payload.stream {
        // 流式响应
        handle_stream_request(
            provider,
            &request_body,
            &payload.model,
            input_tokens,
            cache_read_tokens,
            thinking_enabled,
            tool_name_map,
            state.request_log.clone(),
            state.prompt_cache.clone(),
            session_fp,
            start_time,
            caller_name,
            thinking_effort,
        )
        .await
    } else {
        // 非流式响应：仅在配置开启时提取 thinking 块
        let extract_thinking = state.extract_thinking && thinking_enabled;
        handle_non_stream_request(
            provider,
            &request_body,
            &payload.model,
            input_tokens,
            cache_read_tokens,
            extract_thinking,
            tool_name_map,
            state.request_log.clone(),
            state.prompt_cache.clone(),
            session_fp,
            start_time,
            caller_name,
            thinking_effort,
        )
        .await
    }
}

/// 处理流式请求
async fn handle_stream_request(
    provider: std::sync::Arc<crate::kiro::provider::KiroProvider>,
    request_body: &str,
    model: &str,
    input_tokens: i32,
    cache_read_tokens: i32,
    thinking_enabled: bool,
    tool_name_map: std::collections::HashMap<String, String>,
    request_log: RequestLogStore,
    prompt_cache: Arc<PromptCacheTracker>,
    session_fp: u64,
    start_time: Instant,
    caller_name: Option<String>,
    thinking_effort: Option<String>,
) -> Response {
    // 调用 Kiro API（支持多凭据故障转移）
    let (response, credential_id) = match provider.call_api_stream(request_body).await {
        Ok(resp) => resp,
        Err(e) => {
            let duration_ms = start_time.elapsed().as_millis() as u64;
            let log = request_log.clone();
            let model_owned = model.to_string();
            tokio::spawn(async move {
                log.push(RequestRecord {
                    model: model_owned,
                    input_tokens,
                    output_tokens: 0,
                    cache_read_tokens,
                    ttft_ms: None,
                    duration_ms,
                    timestamp: now_ms(),
                    stream: true,
                    credential_id: None,
                    success: false,
                    credits: 0.0,
                    caller: caller_name,
                    thinking_effort,
                });
            });
            return map_provider_error(e);
        }
    };

    // 创建流处理上下文
    let mut ctx = StreamContext::new_with_thinking_and_start(
        model,
        input_tokens,
        cache_read_tokens,
        thinking_enabled,
        tool_name_map,
        start_time,
    );

    // 生成初始事件
    let initial_events = ctx.generate_initial_events();

    // 创建 SSE 流（带请求记录）
    let stream = create_sse_stream(
        response,
        ctx,
        initial_events,
        request_log,
        model.to_string(),
        input_tokens,
        cache_read_tokens,
        credential_id,
        prompt_cache,
        session_fp,
        start_time,
        caller_name,
        thinking_effort,
    );

    // 返回 SSE 响应
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::CONNECTION, "keep-alive")
        .body(Body::from_stream(stream))
        .unwrap()
}

/// Ping 事件间隔（25秒）
const PING_INTERVAL_SECS: u64 = 25;

/// 创建 ping 事件的 SSE 字符串
fn create_ping_sse() -> Bytes {
    Bytes::from("event: ping\ndata: {\"type\": \"ping\"}\n\n")
}

/// 创建 SSE 事件流
fn create_sse_stream(
    response: reqwest::Response,
    ctx: StreamContext,
    initial_events: Vec<SseEvent>,
    request_log: RequestLogStore,
    model: String,
    input_tokens: i32,
    cache_read_tokens: i32,
    credential_id: u64,
    prompt_cache: Arc<PromptCacheTracker>,
    session_fp: u64,
    start_time: Instant,
    caller_name: Option<String>,
    thinking_effort: Option<String>,
) -> impl Stream<Item = Result<Bytes, Infallible>> {
    // 先发送初始事件
    let initial_stream = stream::iter(
        initial_events
            .into_iter()
            .map(|e| Ok(Bytes::from(e.to_sse_string()))),
    );

    // 然后处理 Kiro 响应流，同时每25秒发送 ping 保活
    let body_stream = response.bytes_stream();

    let processing_stream = stream::unfold(
        (body_stream, ctx, EventStreamDecoder::new(), false, interval(Duration::from_secs(PING_INTERVAL_SECS)), false, request_log, model, input_tokens, cache_read_tokens, credential_id, prompt_cache, session_fp, start_time, caller_name, thinking_effort),
        |(mut body_stream, mut ctx, mut decoder, finished, mut ping_interval, first_token_received, request_log, model, input_tokens, cache_read_tokens, credential_id, prompt_cache, session_fp, start_time, caller_name, thinking_effort)| async move {
            if finished {
                return None;
            }

            // 使用 select! 同时等待数据和 ping 定时器
            tokio::select! {
                // 处理数据流
                chunk_result = body_stream.next() => {
                    match chunk_result {
                        Some(Ok(chunk)) => {
                            // 解码事件
                            if let Err(e) = decoder.feed(&chunk) {
                                tracing::warn!("缓冲区溢出: {}", e);
                            }

                            let mut events = Vec::new();
                            let mut got_first_token = first_token_received;
                            for result in decoder.decode_iter() {
                                match result {
                                    Ok(frame) => {
                                        if let Ok(event) = Event::from_frame(frame) {
                                            if !got_first_token {
                                                if matches!(&event, Event::AssistantResponse(_)) {
                                                    got_first_token = true;
                                                }
                                            }
                                            let sse_events = ctx.process_kiro_event(&event);
                                            events.extend(sse_events);
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!("解码事件失败: {}", e);
                                    }
                                }
                            }

                            // 转换为 SSE 字节流
                            let bytes: Vec<Result<Bytes, Infallible>> = events
                                .into_iter()
                                .map(|e| Ok(Bytes::from(e.to_sse_string())))
                                .collect();

                            Some((stream::iter(bytes), (body_stream, ctx, decoder, false, ping_interval, got_first_token, request_log, model, input_tokens, cache_read_tokens, credential_id, prompt_cache, session_fp, start_time, caller_name, thinking_effort)))
                        }
                        Some(Err(e)) => {
                            tracing::error!("读取响应流失败: {}", e);
                            let output_tokens = ctx.output_tokens();
                            let ttft_ms = ctx.ttft_ms();
                            let credits = ctx.credits();
                            let final_input_tokens = ctx.context_input_tokens.unwrap_or(input_tokens);
                            if ctx.context_input_tokens.is_some() {
                                prompt_cache.update_actual_tokens(session_fp, final_input_tokens);
                            }
                            // 发送最终事件并结束
                            let final_events = ctx.generate_final_events();
                            let bytes: Vec<Result<Bytes, Infallible>> = final_events
                                .into_iter()
                                .map(|e| Ok(Bytes::from(e.to_sse_string())))
                                .collect();
                            // 异步记录
                            let duration_ms = start_time.elapsed().as_millis() as u64;
                            tokio::spawn(async move {
                                request_log.push(RequestRecord {
                                    model,
                                    input_tokens: final_input_tokens,
                                    output_tokens,
                                    cache_read_tokens: cache_read_tokens.min(final_input_tokens),
                                    ttft_ms,
                                    duration_ms,
                                    timestamp: now_ms(),
                                    stream: true,
                                    credential_id: Some(credential_id),
                                    success: false,
                                    credits,
                                    caller: caller_name,
                                    thinking_effort,
                                });
                            });
                            Some((stream::iter(bytes), (body_stream, ctx, decoder, true, ping_interval, first_token_received, RequestLogStore::new(), String::new(), 0, 0, 0, prompt_cache, session_fp, start_time, None, None)))
                        }
                        None => {
                            // 流结束，发送最终事件
                            let output_tokens = ctx.output_tokens();
                            let ttft_ms = ctx.ttft_ms();
                            let credits = ctx.credits();
                            let final_input_tokens = ctx.context_input_tokens.unwrap_or(input_tokens);
                            if ctx.context_input_tokens.is_some() {
                                prompt_cache.update_actual_tokens(session_fp, final_input_tokens);
                            }
                            let final_events = ctx.generate_final_events();
                            let bytes: Vec<Result<Bytes, Infallible>> = final_events
                                .into_iter()
                                .map(|e| Ok(Bytes::from(e.to_sse_string())))
                                .collect();
                            // 异步记录
                            let duration_ms = start_time.elapsed().as_millis() as u64;
                            tokio::spawn(async move {
                                request_log.push(RequestRecord {
                                    model,
                                    input_tokens: final_input_tokens,
                                    output_tokens,
                                    cache_read_tokens: cache_read_tokens.min(final_input_tokens),
                                    ttft_ms,
                                    duration_ms,
                                    timestamp: now_ms(),
                                    stream: true,
                                    credential_id: Some(credential_id),
                                    success: true,
                                    credits,
                                    caller: caller_name,
                                    thinking_effort,
                                });
                            });
                            Some((stream::iter(bytes), (body_stream, ctx, decoder, true, ping_interval, first_token_received, RequestLogStore::new(), String::new(), 0, 0, 0, prompt_cache, session_fp, start_time, None, None)))
                        }
                    }
                }
                // 发送 ping 保活
                _ = ping_interval.tick() => {
                    tracing::trace!("发送 ping 保活事件");
                    let bytes: Vec<Result<Bytes, Infallible>> = vec![Ok(create_ping_sse())];
                    Some((stream::iter(bytes), (body_stream, ctx, decoder, false, ping_interval, first_token_received, request_log, model, input_tokens, cache_read_tokens, credential_id, prompt_cache, session_fp, start_time, caller_name, thinking_effort)))
                }
            }
        },
    )
    .flatten();

    initial_stream.chain(processing_stream)
}

use super::converter::get_context_window_size;

/// 处理非流式请求
async fn handle_non_stream_request(
    provider: std::sync::Arc<crate::kiro::provider::KiroProvider>,
    request_body: &str,
    model: &str,
    input_tokens: i32,
    cache_read_tokens: i32,
    thinking_enabled: bool,
    tool_name_map: std::collections::HashMap<String, String>,
    request_log: RequestLogStore,
    prompt_cache: Arc<PromptCacheTracker>,
    session_fp: u64,
    start_time: Instant,
    caller_name: Option<String>,
    thinking_effort: Option<String>,
) -> Response {
    // 调用 Kiro API（支持多凭据故障转移）
    let (response, credential_id) = match provider.call_api(request_body).await {
        Ok(resp) => resp,
        Err(e) => {
            let duration_ms = start_time.elapsed().as_millis() as u64;
            let model_owned = model.to_string();
            tokio::spawn(async move {
                request_log.push(RequestRecord {
                    model: model_owned,
                    input_tokens,
                    output_tokens: 0,
                    cache_read_tokens,
                    ttft_ms: None,
                    duration_ms,
                    timestamp: now_ms(),
                    stream: false,
                    credential_id: None,
                    success: false,
                    credits: 0.0,
                    caller: caller_name,
                    thinking_effort,
                });
            });
            return map_provider_error(e);
        }
    };

    // 读取响应体
    let body_bytes = match response.bytes().await {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::error!("读取响应体失败: {}", e);
            return (
                StatusCode::BAD_GATEWAY,
                Json(ErrorResponse::new(
                    "api_error",
                    format!("读取响应失败: {}", e),
                )),
            )
                .into_response();
        }
    };

    // 解析事件流
    let mut decoder = EventStreamDecoder::new();
    if let Err(e) = decoder.feed(&body_bytes) {
        tracing::warn!("缓冲区溢出: {}", e);
    }

    let mut text_content = String::new();
    let mut tool_uses: Vec<serde_json::Value> = Vec::new();
    let mut has_tool_use = false;
    let mut stop_reason = "end_turn".to_string();
    // 从 contextUsageEvent 计算的实际输入 tokens
    let mut context_input_tokens: Option<i32> = None;
    let mut credits: f64 = 0.0;

    // 收集工具调用的增量 JSON
    let mut tool_json_buffers: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    for result in decoder.decode_iter() {
        match result {
            Ok(frame) => {
                if let Ok(event) = Event::from_frame(frame) {
                    match event {
                        Event::AssistantResponse(resp) => {
                            text_content.push_str(&resp.content);
                        }
                        Event::ToolUse(tool_use) => {
                            has_tool_use = true;

                            // 累积工具的 JSON 输入
                            let buffer = tool_json_buffers
                                .entry(tool_use.tool_use_id.clone())
                                .or_insert_with(String::new);
                            buffer.push_str(&tool_use.input);

                            // 如果是完整的工具调用，添加到列表
                            if tool_use.stop {
                                let input: serde_json::Value = if buffer.is_empty() {
                                    serde_json::json!({})
                                } else {
                                    serde_json::from_str(buffer).unwrap_or_else(|e| {
                                        tracing::warn!(
                                            "工具输入 JSON 解析失败: {}, tool_use_id: {}",
                                            e,
                                            tool_use.tool_use_id
                                        );
                                        serde_json::json!({})
                                    })
                                };

                                let original_name = tool_name_map
                                    .get(&tool_use.name)
                                    .cloned()
                                    .unwrap_or_else(|| tool_use.name.clone());

                                tool_uses.push(json!({
                                    "type": "tool_use",
                                    "id": tool_use.tool_use_id,
                                    "name": original_name,
                                    "input": input
                                }));
                            }
                        }
                        Event::ContextUsage(context_usage) => {
                            // 从上下文使用百分比计算实际的 input_tokens
                            let window_size = get_context_window_size(model);
                            let actual_input_tokens =
                                (context_usage.context_usage_percentage * (window_size as f64)
                                    / 100.0) as i32;
                            context_input_tokens = Some(actual_input_tokens);
                            // 上下文使用量达到 100% 时，设置 stop_reason 为 model_context_window_exceeded
                            if context_usage.context_usage_percentage >= 100.0 {
                                stop_reason = "model_context_window_exceeded".to_string();
                            }
                            tracing::debug!(
                                "收到 contextUsageEvent: {}%, 计算 input_tokens: {}",
                                context_usage.context_usage_percentage,
                                actual_input_tokens
                            );
                        }
                        Event::Exception { exception_type, .. } => {
                            if exception_type == "ContentLengthExceededException" {
                                stop_reason = "max_tokens".to_string();
                            }
                        }
                        Event::Metering(metering) => {
                            credits += metering.usage;
                        }
                        _ => {}
                    }
                }
            }
            Err(e) => {
                tracing::warn!("解码事件失败: {}", e);
            }
        }
    }

    // 确定 stop_reason
    if has_tool_use && stop_reason == "end_turn" {
        stop_reason = "tool_use".to_string();
    }

    // 构建响应内容
    let mut content: Vec<serde_json::Value> = Vec::new();

    if thinking_enabled {
        // 从完整文本中提取 thinking 块
        let (thinking, remaining_text) =
            super::stream::extract_thinking_from_complete_text(&text_content);

        if let Some(thinking_text) = thinking {
            content.push(json!({
                "type": "thinking",
                "thinking": thinking_text
            }));
        }

        if !remaining_text.is_empty() {
            content.push(json!({
                "type": "text",
                "text": remaining_text
            }));
        }
    } else if !text_content.is_empty() {
        content.push(json!({
            "type": "text",
            "text": text_content
        }));
    }

    content.extend(tool_uses);

    // 估算输出 tokens
    let output_tokens = token::estimate_output_tokens(&content);

    // 使用从 contextUsageEvent 计算的 input_tokens，如果没有则使用估算值
    let final_input_tokens = context_input_tokens.unwrap_or(input_tokens);

    if context_input_tokens.is_some() {
        prompt_cache.update_actual_tokens(session_fp, final_input_tokens);
    }

    // 异步记录请求
    {
        let duration_ms = start_time.elapsed().as_millis() as u64;
        let model_owned = model.to_string();
        tokio::spawn(async move {
            request_log.push(RequestRecord {
                model: model_owned,
                input_tokens: final_input_tokens,
                output_tokens,
                cache_read_tokens: cache_read_tokens.min(final_input_tokens),
                ttft_ms: None,
                duration_ms,
                timestamp: now_ms(),
                stream: false,
                credential_id: Some(credential_id),
                success: true,
                credits,
                caller: caller_name,
                thinking_effort,
            });
        });
    }

    // 构建 Anthropic 响应
    let response_body = json!({
        "id": format!("msg_{}", Uuid::new_v4().to_string().replace('-', "")),
        "type": "message",
        "role": "assistant",
        "content": content,
        "model": model,
        "stop_reason": stop_reason,
        "stop_sequence": null,
        "usage": {
            "input_tokens": (final_input_tokens - cache_read_tokens.min(final_input_tokens)).max(0),
            "output_tokens": output_tokens,
            "cache_read_input_tokens": cache_read_tokens.min(final_input_tokens),
            "cache_creation_input_tokens": 0
        }
    });

    (StatusCode::OK, Json(response_body)).into_response()
}

/// POST /v1/messages/count_tokens
///
/// 计算消息的 token 数量
pub async fn count_tokens(
    JsonExtractor(payload): JsonExtractor<CountTokensRequest>,
) -> impl IntoResponse {
    tracing::info!(
        model = %payload.model,
        message_count = %payload.messages.len(),
        "Received POST /v1/messages/count_tokens request"
    );

    let total_tokens = token::count_all_tokens(
        payload.model,
        payload.system,
        payload.messages,
        payload.tools,
    ) as i32;

    Json(CountTokensResponse {
        input_tokens: total_tokens.max(1) as i32,
    })
}
