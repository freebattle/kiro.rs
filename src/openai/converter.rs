//! Responses / OpenAI 消息 → Anthropic MessagesRequest

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::anthropic::types::{Message, MessagesRequest, SystemMessage, Tool};

use super::input::extract_additional_tools;
use super::types::{OpenAIMessage, ResponsesRequest, ResponsesTool};

const DEFAULT_MAX_OUTPUT_TOKENS: i32 = 16384;

/// 将 Responses 请求 + 展开后的消息转为 Anthropic MessagesRequest
pub fn responses_to_anthropic(
    req: &ResponsesRequest,
    messages: &[OpenAIMessage],
) -> Result<MessagesRequest, String> {
    let mut system_parts: Vec<String> = Vec::new();
    let mut anthropic_messages: Vec<Message> = Vec::new();
    let mut pending_tool_results: Vec<Value> = Vec::new();

    let flush_tool_results = |pending: &mut Vec<Value>, out: &mut Vec<Message>| {
        if pending.is_empty() {
            return;
        }
        out.push(Message {
            role: "user".to_string(),
            content: Value::Array(std::mem::take(pending)),
        });
    };

    for msg in messages {
        match msg.role.as_str() {
            "system" | "developer" => {
                let t = msg.text_content();
                if !t.trim().is_empty() {
                    system_parts.push(t);
                }
            }
            "user" => {
                flush_tool_results(&mut pending_tool_results, &mut anthropic_messages);
                anthropic_messages.push(Message {
                    role: "user".to_string(),
                    content: openai_user_content_to_anthropic(&msg.content),
                });
            }
            "assistant" => {
                flush_tool_results(&mut pending_tool_results, &mut anthropic_messages);
                let mut blocks: Vec<Value> = Vec::new();
                let text = msg.text_content();
                if !text.is_empty() {
                    blocks.push(json!({"type":"text","text": text}));
                }
                for tc in &msg.tool_calls {
                    let input: Value = serde_json::from_str(&tc.function.arguments)
                        .unwrap_or_else(|_| json!({}));
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": tc.id,
                        "name": tc.function.name,
                        "input": input
                    }));
                }
                if blocks.is_empty() {
                    // 空 assistant 跳过（避免破坏配对）
                    continue;
                }
                anthropic_messages.push(Message {
                    role: "assistant".to_string(),
                    content: Value::Array(blocks),
                });
            }
            "tool" => {
                let call_id = msg.tool_call_id.clone().unwrap_or_default();
                let content_text = msg.text_content();
                pending_tool_results.push(json!({
                    "type": "tool_result",
                    "tool_use_id": call_id,
                    "content": content_text
                }));
            }
            _ => {}
        }
    }
    flush_tool_results(&mut pending_tool_results, &mut anthropic_messages);

    if anthropic_messages.is_empty() {
        return Err("input must contain at least one message".to_string());
    }
    if !anthropic_messages.iter().any(|m| m.role == "user") {
        return Err("input must contain at least one user message".to_string());
    }

    let system = if system_parts.is_empty() {
        None
    } else {
        Some(vec![SystemMessage {
            text: system_parts.join("\n"),
            cache_control: None,
        }])
    };

    // 合并顶层 tools + Codex Lite additional_tools（input 内载体）
    let mut effective_tools: Vec<ResponsesTool> = req.tools.clone().unwrap_or_default();
    effective_tools.extend(extract_additional_tools(&req.input));
    let tools = if effective_tools.is_empty() {
        None
    } else {
        let converted = convert_tools(&effective_tools);
        if converted.is_empty() {
            None
        } else {
            Some(converted)
        }
    };
    let max_tokens = req.max_output_tokens.unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS);

    Ok(MessagesRequest {
        model: if req.model.trim().is_empty() {
            "claude-sonnet-4.5".to_string()
        } else {
            req.model.clone()
        },
        max_tokens,
        messages: anthropic_messages,
        stream: req.stream,
        system,
        tools: tools.clone(),
        tool_choice: sanitize_tool_choice(req.tool_choice.as_ref(), tools.as_ref()),
        thinking: None,
        output_config: req.reasoning.as_ref().and_then(|reasoning| {
            reasoning.effort.as_ref().map(|effort| crate::anthropic::types::OutputConfig {
                effort: effort.clone(),
            })
        }),
        metadata: None,
    })
}

/// 丢弃指向服务端工具（web_search 等）的 tool_choice，避免污染上游。
fn sanitize_tool_choice(
    choice: Option<&Value>,
    tools: Option<&Vec<Tool>>,
) -> Option<Value> {
    let Some(choice) = choice else {
        return None;
    };
    match choice {
        Value::String(s) => {
            if matches!(s.as_str(), "auto" | "none" | "required" | "any") {
                Some(choice.clone())
            } else if tools
                .map(|ts| ts.iter().any(|t| t.name == *s))
                .unwrap_or(false)
            {
                Some(choice.clone())
            } else {
                Some(json!("auto"))
            }
        }
        Value::Object(map) => {
            let typ = map.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match typ {
                "function" | "tool" | "custom" => {
                    let name = map
                        .get("name")
                        .or_else(|| map.get("function").and_then(|f| f.get("name")))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if name.is_empty()
                        || tools
                            .map(|ts| ts.iter().any(|t| t.name == name))
                            .unwrap_or(false)
                    {
                        Some(choice.clone())
                    } else {
                        Some(json!("auto"))
                    }
                }
                "auto" | "none" | "required" | "any" => Some(choice.clone()),
                // web_search / image_generation 等
                _ => Some(json!("auto")),
            }
        }
        _ => Some(json!("auto")),
    }
}

fn openai_user_content_to_anthropic(content: &Value) -> Value {
    match content {
        Value::String(s) => Value::String(s.clone()),
        Value::Array(parts) => {
            let mut blocks = Vec::new();
            for p in parts {
                let Some(obj) = p.as_object() else { continue };
                let ptype = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match ptype {
                    "input_text" | "text" | "output_text" => {
                        if let Some(t) = obj.get("text").and_then(|v| v.as_str()) {
                            blocks.push(json!({"type":"text","text": t}));
                        }
                    }
                    "input_image" | "image" | "image_url" => {
                        // 尽量保留图片：Anthropic 期望 type=image + source
                        if let Some(source) = extract_image_source(obj) {
                            blocks.push(json!({
                                "type": "image",
                                "source": source
                            }));
                        } else if let Some(url) = obj
                            .get("image_url")
                            .and_then(|v| v.as_object())
                            .and_then(|o| o.get("url"))
                            .and_then(|v| v.as_str())
                            .or_else(|| obj.get("url").and_then(|v| v.as_str()))
                        {
                            // data URL 可拆 base64；其它 URL 暂以文本占位
                            if let Some((media, data)) = parse_data_url(url) {
                                blocks.push(json!({
                                    "type": "image",
                                    "source": {
                                        "type": "base64",
                                        "media_type": media,
                                        "data": data
                                    }
                                }));
                            } else {
                                blocks.push(json!({
                                    "type": "text",
                                    "text": format!("[image: {url}]")
                                }));
                            }
                        }
                    }
                    _ => {
                        if let Some(t) = obj.get("text").and_then(|v| v.as_str()) {
                            if !t.is_empty() {
                                blocks.push(json!({"type":"text","text": t}));
                            }
                        }
                    }
                }
            }
            if blocks.is_empty() {
                Value::String(String::new())
            } else if blocks.len() == 1
                && blocks[0].get("type").and_then(|v| v.as_str()) == Some("text")
            {
                blocks[0]
                    .get("text")
                    .cloned()
                    .unwrap_or(Value::String(String::new()))
            } else {
                Value::Array(blocks)
            }
        }
        other => Value::String(other.as_str().unwrap_or("").to_string()),
    }
}

fn extract_image_source(obj: &serde_json::Map<String, Value>) -> Option<Value> {
    obj.get("source").cloned()
}

fn parse_data_url(url: &str) -> Option<(String, String)> {
    // data:image/png;base64,xxxx
    let rest = url.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    if !meta.contains("base64") {
        return None;
    }
    let media = meta.split(';').next()?.to_string();
    Some((media, data.to_string()))
}

/// Codex custom/freeform 工具降级为 function 时的参数 schema。
/// 对齐 sub2api: 输入包进 `{"input":"..."}`。
const CUSTOM_TOOL_INPUT_SCHEMA: &str = r#"{"type":"object","properties":{"input":{"type":"string","description":"The raw input for this tool, passed through verbatim."}},"required":["input"]}"#;

const TOOL_SEARCH_NAME: &str = "tool_search";
const TOOL_SEARCH_SCHEMA: &str = r#"{"type":"object","properties":{"query":{"type":"string","description":"Search query for tools or connectors to load."},"limit":{"type":"integer","description":"Maximum number of tool groups to return."}},"required":["query"]}"#;

/// 将 Responses tools 转为 Anthropic/Kiro 可接受的 function 工具列表。
///
/// 策略对齐 sub2api chat bridge：
/// - function: 透传
/// - custom: 降级为 function（Codex exec 等）
/// - tool_search: 降级为同名 function 代理
/// - namespace: 子工具摊平为 `namespace__name`
/// - web_search / image_generation / local_shell 等服务端工具：丢弃
pub fn convert_tools(tools: &[ResponsesTool]) -> Vec<Tool> {
    let mut out = Vec::new();
    let mut tool_search_added = false;

    for t in tools {
        match t.tool_type.as_str() {
            "function" | "" => {
                if t.name.trim().is_empty() {
                    continue;
                }
                out.push(tool_from_function(
                    &t.name,
                    t.description.as_deref().unwrap_or(""),
                    t.parameters.as_ref(),
                ));
            }
            "custom" => {
                if t.name.trim().is_empty() {
                    continue;
                }
                // custom 无 parameters 时用 freeform input schema；有则尊重原 schema
                let params = t
                    .parameters
                    .clone()
                    .or_else(|| serde_json::from_str(CUSTOM_TOOL_INPUT_SCHEMA).ok());
                out.push(tool_from_function(
                    &t.name,
                    t.description.as_deref().unwrap_or(""),
                    params.as_ref(),
                ));
            }
            "tool_search" => {
                if tool_search_added {
                    continue;
                }
                // 与客户端同名 function 冲突时跳过代理（避免劫持）
                if tools.iter().any(|x| {
                    matches!(x.tool_type.as_str(), "function" | "custom")
                        && x.name == TOOL_SEARCH_NAME
                }) {
                    tracing::warn!(
                        "skip tool_search proxy: conflicts with declared tool named tool_search"
                    );
                    continue;
                }
                let params: Value = serde_json::from_str(TOOL_SEARCH_SCHEMA).unwrap_or(json!({
                    "type": "object",
                    "properties": {}
                }));
                out.push(tool_from_function(
                    TOOL_SEARCH_NAME,
                    "Search and load Codex tools, plugins, connectors, and MCP namespaces for the current task.",
                    Some(&params),
                ));
                tool_search_added = true;
            }
            "namespace" => {
                if t.name.trim().is_empty() {
                    continue;
                }
                let children = if !t.tools.is_empty() {
                    &t.tools
                } else {
                    &t.children
                };
                for child in children {
                    if !matches!(child.tool_type.as_str(), "function" | "custom" | "") {
                        continue;
                    }
                    if child.name.trim().is_empty() {
                        continue;
                    }
                    let flat = format!("{}__{}", t.name, child.name);
                    let params = if child.tool_type == "custom" && child.parameters.is_none() {
                        serde_json::from_str(CUSTOM_TOOL_INPUT_SCHEMA).ok()
                    } else {
                        child.parameters.clone()
                    };
                    out.push(tool_from_function(
                        &flat,
                        child.description.as_deref().unwrap_or(""),
                        params.as_ref(),
                    ));
                }
            }
            // 服务端工具：Kiro 上游无对应能力，丢弃（与 sub2api 一致）
            "web_search"
            | "image_generation"
            | "local_shell"
            | "code_interpreter"
            | "file_search" => {}
            other => {
                // 未知类型：若带 name 则尽量当 function 透传，避免 Codex 全挂
                if !t.name.trim().is_empty() {
                    tracing::debug!(
                        tool_type = other,
                        name = %t.name,
                        "passthrough unknown responses tool as function"
                    );
                    out.push(tool_from_function(
                        &t.name,
                        t.description.as_deref().unwrap_or(""),
                        t.parameters.as_ref(),
                    ));
                }
            }
        }
    }
    out
}

fn tool_from_function(name: &str, description: &str, parameters: Option<&Value>) -> Tool {
    let mut input_schema = HashMap::new();
    if let Some(Value::Object(map)) = parameters.cloned() {
        for (k, v) in map {
            input_schema.insert(k, v);
        }
    } else {
        input_schema.insert("type".to_string(), json!("object"));
        input_schema.insert("properties".to_string(), json!({}));
    }
    // Kiro 对空 description 可能返回 Invalid tool use format
    let description = if description.trim().is_empty() {
        format!("Tool {name}")
    } else {
        description.to_string()
    };
    Tool {
        tool_type: None,
        name: name.to_string(),
        description,
        input_schema,
        max_uses: None,
        cache_control: None,
    }
}

/// 从请求中收集 custom 工具名（含 additional_tools 与 namespace 子 custom）。
pub fn collect_custom_tool_names(req: &ResponsesRequest) -> std::collections::HashSet<String> {
    let mut names = std::collections::HashSet::new();
    let mut all = req.tools.clone().unwrap_or_default();
    all.extend(extract_additional_tools(&req.input));
    collect_custom_names_from_tools(&all, &mut names);
    names
}

fn collect_custom_names_from_tools(
    tools: &[ResponsesTool],
    names: &mut std::collections::HashSet<String>,
) {
    for t in tools {
        match t.tool_type.as_str() {
            "custom" => {
                if !t.name.trim().is_empty() {
                    names.insert(t.name.clone());
                }
            }
            "namespace" => {
                let children = if !t.tools.is_empty() {
                    &t.tools
                } else {
                    &t.children
                };
                for child in children {
                    if child.tool_type == "custom" && !child.name.trim().is_empty() {
                        // 摊平名与裸名都记：模型可能回摊平名
                        names.insert(child.name.clone());
                        names.insert(format!("{}__{}", t.name, child.name));
                    }
                }
            }
            _ => {}
        }
    }
}

/// 从降级 function 调用的 arguments 中还原 custom freeform input。
pub fn extract_custom_tool_call_input(arguments: &str) -> String {
    let trimmed = arguments.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Ok(obj) = serde_json::from_str::<serde_json::Map<String, Value>>(trimmed) {
        if let Some(input) = obj.get("input") {
            match input {
                Value::String(s) => return s.clone(),
                other => return other.to_string(),
            }
        }
    }
    // 模型未按 schema 输出时原样回传
    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openai::types::{OpenAIMessage, ResponsesRequest, ToolCall};

    #[test]
    fn test_reasoning_effort_converts_to_output_config() {
        let req = ResponsesRequest {
            model: "gpt-5.6".to_string(),
            input: json!("hi"),
            instructions: None,
            stream: false,
            tools: None,
            tool_choice: None,
            previous_response_id: None,
            store: None,
            temperature: None,
            max_output_tokens: None,
            reasoning: Some(crate::openai::types::ReasoningConfig {
                effort: Some("low".to_string()),
            }),
            metadata: None,
        };
        let out = responses_to_anthropic(&req, &[OpenAIMessage::user_text("hi")]).unwrap();

        assert_eq!(out.output_config.unwrap().effort, "low");
    }

    #[test]
    fn test_responses_to_anthropic_basic() {
        let req = ResponsesRequest {
            model: "gpt-5.6".to_string(),
            input: json!("hi"),
            instructions: None,
            stream: false,
            tools: None,
            tool_choice: None,
            previous_response_id: None,
            store: None,
            temperature: None,
            max_output_tokens: Some(1024),
            reasoning: None,
            metadata: None,
        };
        let messages = vec![
            OpenAIMessage::system_text("be nice"),
            OpenAIMessage::user_text("hi"),
        ];
        let out = responses_to_anthropic(&req, &messages).unwrap();
        assert_eq!(out.model, "gpt-5.6");
        assert_eq!(out.max_tokens, 1024);
        assert_eq!(out.system.as_ref().unwrap()[0].text, "be nice");
        assert_eq!(out.messages.len(), 1);
        assert_eq!(out.messages[0].role, "user");
    }

    #[test]
    fn test_tool_call_and_result_pairing() {
        let req = ResponsesRequest {
            model: "claude-sonnet-4.5".to_string(),
            input: json!([]),
            instructions: None,
            stream: false,
            tools: Some(vec![ResponsesTool {
                tool_type: "function".to_string(),
                name: "Bash".to_string(),
                description: Some("run".to_string()),
                parameters: Some(json!({"type":"object","properties":{}})),
                tools: vec![],
                children: vec![],
            }]),
            tool_choice: None,
            previous_response_id: None,
            store: None,
            temperature: None,
            max_output_tokens: None,
            reasoning: None,
            metadata: None,
        };
        let messages = vec![
            OpenAIMessage::user_text("run ls"),
            OpenAIMessage::assistant_tool_calls(vec![ToolCall::function(
                "call_1",
                "Bash",
                r#"{"command":"ls"}"#,
            )]),
            OpenAIMessage::tool_result("call_1", "a.txt"),
            OpenAIMessage::user_text("ok"),
        ];
        let out = responses_to_anthropic(&req, &messages).unwrap();
        // user, assistant(tool_use), user(tool_result), user(ok)
        // tool_result is user content array; final user text separate
        assert!(out.messages.iter().any(|m| m.role == "assistant"));
        let tool_user = out
            .messages
            .iter()
            .find(|m| {
                m.role == "user"
                    && m.content
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_result"))
                        })
                        .unwrap_or(false)
            })
            .expect("tool_result user message");
        let arr = tool_user.content.as_array().unwrap();
        assert_eq!(arr[0]["tool_use_id"], "call_1");
        assert_eq!(out.tools.as_ref().unwrap()[0].name, "Bash");
    }

    #[test]
    fn test_convert_tools_codex_shapes() {
        use crate::openai::types::ResponsesTool;
        let tools = vec![
            ResponsesTool {
                tool_type: "web_search".to_string(),
                name: String::new(),
                description: None,
                parameters: None,
                tools: vec![],
                children: vec![],
            },
            ResponsesTool {
                tool_type: "custom".to_string(),
                name: "exec".to_string(),
                description: Some("Run command".to_string()),
                parameters: None,
                tools: vec![],
                children: vec![],
            },
            ResponsesTool {
                tool_type: "function".to_string(),
                name: "wait".to_string(),
                description: Some("wait".to_string()),
                parameters: Some(json!({"type":"object","properties":{}})),
                tools: vec![],
                children: vec![],
            },
            ResponsesTool {
                tool_type: "tool_search".to_string(),
                name: String::new(),
                description: None,
                parameters: None,
                tools: vec![],
                children: vec![],
            },
            ResponsesTool {
                tool_type: "namespace".to_string(),
                name: "collaboration".to_string(),
                description: None,
                parameters: None,
                tools: vec![ResponsesTool {
                    tool_type: "function".to_string(),
                    name: "send_message".to_string(),
                    description: Some("send".to_string()),
                    parameters: Some(json!({"type":"object"})),
                    tools: vec![],
                    children: vec![],
                }],
                children: vec![],
            },
        ];
        let converted = convert_tools(&tools);
        let names: Vec<_> = converted.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"exec"));
        assert!(names.contains(&"wait"));
        assert!(names.contains(&"tool_search"));
        assert!(names.contains(&"collaboration__send_message"));
        assert!(!names.iter().any(|n| n.is_empty()));
        // web_search dropped
        assert_eq!(names.len(), 4);
        let exec = converted.iter().find(|t| t.name == "exec").unwrap();
        assert_eq!(exec.input_schema.get("type").and_then(|v| v.as_str()), Some("object"));
        assert!(exec.input_schema.get("properties").unwrap().get("input").is_some());
    }

    #[test]
    fn test_tool_string_and_web_search_deserialize() {
        use crate::openai::types::ResponsesTool;
        let t: ResponsesTool = serde_json::from_value(json!("exec")).unwrap();
        assert_eq!(t.tool_type, "custom");
        assert_eq!(t.name, "exec");
        let ws: ResponsesTool = serde_json::from_value(json!({"type":"web_search"})).unwrap();
        assert_eq!(ws.tool_type, "web_search");
    }


    #[test]
    fn test_additional_tools_merged_into_tools() {
        let req = ResponsesRequest {
            model: "gpt-5.6-luna".to_string(),
            input: json!([
                {
                    "type":"additional_tools",
                    "role":"developer",
                    "tools":[
                        {"type":"custom","name":"exec","description":"run"},
                        {"type":"function","name":"wait","description":"wait","parameters":{"type":"object"}}
                    ]
                },
                {"type":"message","role":"user","content":"hi"}
            ]),
            instructions: None,
            stream: false,
            tools: None,
            tool_choice: None,
            previous_response_id: None,
            store: None,
            temperature: None,
            max_output_tokens: Some(32),
            reasoning: None,
            metadata: None,
        };
        let messages = vec![OpenAIMessage::user_text("hi")];
        let out = responses_to_anthropic(&req, &messages).unwrap();
        let tools = out.tools.expect("tools");
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"exec"));
        assert!(names.contains(&"wait"));
        let customs = collect_custom_tool_names(&req);
        assert!(customs.contains("exec"));
        assert!(!customs.contains("wait"));
        assert_eq!(extract_custom_tool_call_input(r#"{"input":"Get-Location"}"#), "Get-Location");
    }

}
