//! Responses API input 解析
//!
//! 将 string / array / object 形式的 `input` 转为中间层 OpenAIMessage 列表。

use serde_json::Value;

use super::types::{OpenAIMessage, ResponsesTool, ToolCall};

#[derive(Debug)]
pub enum InputError {
    UnsupportedShape,
    InvalidString(String),
    InvalidArray(String),
}

impl std::fmt::Display for InputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedShape => write!(f, "unsupported input shape"),
            Self::InvalidString(e) => write!(f, "invalid input string: {e}"),
            Self::InvalidArray(e) => write!(f, "invalid input array: {e}"),
        }
    }
}

impl std::error::Error for InputError {}


/// 从 Responses input 中提取 Codex Lite 的 `additional_tools` 载体。
///
/// 新版 Codex 常把运行时工具放在 input 数组项：
/// `{"type":"additional_tools","role":"developer","tools":[...]}`
/// 而不是顶层 `tools` 字段。
pub fn extract_additional_tools(raw: &Value) -> Vec<ResponsesTool> {
    let items = match raw {
        Value::Array(items) => items.as_slice(),
        Value::Object(_) => std::slice::from_ref(raw),
        _ => return Vec::new(),
    };
    let mut out = Vec::new();
    for item in items {
        let Some(obj) = item.as_object() else { continue };
        let typ = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if typ != "additional_tools" {
            continue;
        }
        let Some(tools_val) = obj.get("tools") else { continue };
        match serde_json::from_value::<Vec<ResponsesTool>>(tools_val.clone()) {
            Ok(tools) => out.extend(tools),
            Err(e) => {
                tracing::warn!("[Responses] failed to parse additional_tools: {e}");
            }
        }
    }
    out
}

/// 解析 Responses API 的 input 字段
pub fn parse_responses_input(raw: &Value) -> Result<Vec<OpenAIMessage>, InputError> {
    match raw {
        Value::Null => Ok(Vec::new()),
        Value::String(s) => {
            if s.trim().is_empty() {
                Ok(Vec::new())
            } else {
                Ok(vec![OpenAIMessage::user_text(s)])
            }
        }
        Value::Array(items) => convert_responses_input_items(items),
        Value::Object(_) => convert_responses_input_items(std::slice::from_ref(raw)),
        _ => Err(InputError::UnsupportedShape),
    }
}

fn convert_responses_input_items(items: &[Value]) -> Result<Vec<OpenAIMessage>, InputError> {
    let mut messages: Vec<OpenAIMessage> = Vec::new();
    let mut pending_user_parts: Vec<Value> = Vec::new();

    let flush_pending_user = |pending: &mut Vec<Value>, messages: &mut Vec<OpenAIMessage>| {
        if pending.is_empty() {
            return;
        }
        messages.push(OpenAIMessage {
            role: "user".to_string(),
            content: Value::Array(std::mem::take(pending)),
            tool_calls: Vec::new(),
            tool_call_id: None,
        });
    };

    for item in items {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let typ = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let role = obj.get("role").and_then(|v| v.as_str()).unwrap_or("");

        match typ {
            "additional_tools" => {
                // tools 载体，不转消息（见 extract_additional_tools）
                continue;
            }
            "message" | "" if !role.is_empty() || typ == "message" => {
                flush_pending_user(&mut pending_user_parts, &mut messages);
                if let Some(msg) = build_message_from_input_item(obj, role) {
                    messages.push(msg);
                }
            }
            "function_call_output" | "custom_tool_call_output" | "tool_search_output" | "tool_result" => {
                flush_pending_user(&mut pending_user_parts, &mut messages);
                let call_id = string_field(obj, &["call_id", "tool_call_id"]);
                let out = stringify_arbitrary(obj.get("output"))
                    .or_else(|| stringify_arbitrary(obj.get("content")))
                    .unwrap_or_default();
                messages.push(OpenAIMessage::tool_result(call_id, out));
            }
            "function_call" | "custom_tool_call" | "tool_search_call" => {
                flush_pending_user(&mut pending_user_parts, &mut messages);
                let call_id = string_field(obj, &["call_id", "id"]);
                let mut name = obj.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                // namespace 子工具历史调用：摊平为 namespace__name
                if let Some(ns) = obj.get("namespace").and_then(|v| v.as_str()) {
                    if !ns.is_empty() && !name.is_empty() {
                        name = format!("{ns}__{name}");
                    }
                }
                let arguments = if typ == "custom_tool_call" {
                    // freeform input -> {"input":"..."}
                    let input = obj
                        .get("input")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .or_else(|| stringify_arbitrary(obj.get("input")))
                        .unwrap_or_default();
                    serde_json::json!({"input": input}).to_string()
                } else if typ == "tool_search_call" {
                    name = if name.is_empty() {
                        "tool_search".to_string()
                    } else {
                        name
                    };
                    stringify_arbitrary(obj.get("arguments")).unwrap_or_else(|| "{}".to_string())
                } else {
                    // function_call: namespace 已摊平
                    stringify_arbitrary(obj.get("arguments")).unwrap_or_else(|| "{}".to_string())
                };
                if typ == "tool_search_call" && name.is_empty() {
                    name = "tool_search".to_string();
                }
                let tc = ToolCall::function(call_id, name, arguments);
                // 合并连续 function_call 到同一 assistant 消息（并行 tool）
                if let Some(last) = messages.last_mut() {
                    if last.role == "assistant"
                        && !last.tool_calls.is_empty()
                        && last.text_content().trim().is_empty()
                    {
                        last.tool_calls.push(tc);
                        continue;
                    }
                }
                messages.push(OpenAIMessage::assistant_tool_calls(vec![tc]));
            }
            "input_text" | "text" => {
                if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                    if !text.is_empty() {
                        pending_user_parts.push(serde_json::json!({
                            "type": "input_text",
                            "text": text
                        }));
                    }
                }
            }
            "input_image" | "image" | "image_url" => {
                pending_user_parts.push(Value::Object(obj.clone()));
            }
            "output_text" => {
                flush_pending_user(&mut pending_user_parts, &mut messages);
                if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                    if !text.is_empty() {
                        messages.push(OpenAIMessage::assistant_text(text));
                    }
                }
            }
            _ => {
                if !role.is_empty() {
                    flush_pending_user(&mut pending_user_parts, &mut messages);
                    if let Some(msg) = build_message_from_input_item(obj, role) {
                        messages.push(msg);
                    }
                }
            }
        }
    }

    flush_pending_user(&mut pending_user_parts, &mut messages);
    Ok(messages)
}

fn build_message_from_input_item(
    obj: &serde_json::Map<String, Value>,
    role: &str,
) -> Option<OpenAIMessage> {
    let role = if role.is_empty() { "user" } else { role };

    if let Some(content) = obj.get("content") {
        match content {
            Value::String(s) => {
                return Some(OpenAIMessage {
                    role: role.to_string(),
                    content: Value::String(s.clone()),
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                });
            }
            Value::Array(parts) => {
                let mut out_parts = Vec::new();
                let mut text_only = String::new();
                let mut any_non_text = false;
                for p in parts {
                    let Some(part) = p.as_object() else { continue };
                    let ptype = part.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    match ptype {
                        "input_text" | "text" | "output_text" => {
                            if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                                text_only.push_str(t);
                                out_parts.push(serde_json::json!({
                                    "type": "input_text",
                                    "text": t
                                }));
                            }
                        }
                        "input_image" | "image" | "image_url" => {
                            any_non_text = true;
                            out_parts.push(Value::Object(part.clone()));
                        }
                        _ => {
                            if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                                if !t.is_empty() {
                                    text_only.push_str(t);
                                    out_parts.push(serde_json::json!({
                                        "type": "input_text",
                                        "text": t
                                    }));
                                }
                            }
                        }
                    }
                }
                if !any_non_text {
                    return Some(OpenAIMessage {
                        role: role.to_string(),
                        content: Value::String(text_only),
                        tool_calls: Vec::new(),
                        tool_call_id: None,
                    });
                }
                return Some(OpenAIMessage {
                    role: role.to_string(),
                    content: Value::Array(out_parts),
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                });
            }
            Value::Object(inner) => {
                return build_message_from_input_item(inner, role);
            }
            _ => {}
        }
    }

    if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
        if !text.is_empty() {
            return Some(OpenAIMessage {
                role: role.to_string(),
                content: Value::String(text.to_string()),
                tool_calls: Vec::new(),
                tool_call_id: None,
            });
        }
    }

    None
}

fn stringify_arbitrary(v: Option<&Value>) -> Option<String> {
    match v {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(other) => Some(other.to_string()),
    }
}

fn string_field(obj: &serde_json::Map<String, Value>, keys: &[&str]) -> String {
    for k in keys {
        if let Some(s) = obj.get(*k).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_string_input() {
        let msgs = parse_responses_input(&json!("hello")).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].text_content(), "hello");
    }

    #[test]
    fn test_parse_array_message_input() {
        let raw = json!([
            {"type":"message","role":"user","content":"hi"},
            {"type":"message","role":"assistant","content":"yo"},
            {"type":"message","role":"user","content":"again"}
        ]);
        let msgs = parse_responses_input(&raw).unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].text_content(), "hi");
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[1].text_content(), "yo");
        assert_eq!(msgs[2].text_content(), "again");
    }

    #[test]
    fn test_parse_function_call_and_output() {
        let raw = json!([
            {"type":"message","role":"user","content":"run ls"},
            {
                "type":"function_call",
                "call_id":"call_1",
                "name":"Bash",
                "arguments":"{\"command\":\"ls\"}"
            },
            {
                "type":"function_call_output",
                "call_id":"call_1",
                "output":"a.txt\nb.txt"
            }
        ]);
        let msgs = parse_responses_input(&raw).unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[1].tool_calls.len(), 1);
        assert_eq!(msgs[1].tool_calls[0].id, "call_1");
        assert_eq!(msgs[1].tool_calls[0].function.name, "Bash");
        assert_eq!(msgs[2].role, "tool");
        assert_eq!(msgs[2].tool_call_id.as_deref(), Some("call_1"));
        assert_eq!(msgs[2].text_content(), "a.txt\nb.txt");
    }

    #[test]
    fn test_parse_parallel_function_calls_merge() {
        let raw = json!([
            {"type":"function_call","call_id":"c1","name":"Read","arguments":"{}"},
            {"type":"function_call","call_id":"c2","name":"Glob","arguments":"{}"}
        ]);
        let msgs = parse_responses_input(&raw).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].tool_calls.len(), 2);
        assert_eq!(msgs[0].tool_calls[0].id, "c1");
        assert_eq!(msgs[0].tool_calls[1].id, "c2");
    }

    #[test]
    fn test_parse_input_text_parts() {
        let raw = json!([
            {"type":"input_text","text":"hello "},
            {"type":"input_text","text":"world"}
        ]);
        let msgs = parse_responses_input(&raw).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].text_content(), "hello world");
    }

    #[test]
    fn test_tool_flat_and_nested_deserialize() {
        use super::super::types::ResponsesTool;
        let nested: ResponsesTool = serde_json::from_value(json!({
            "type":"function",
            "function":{"name":"Bash","description":"run","parameters":{"type":"object"}}
        }))
        .unwrap();
        assert_eq!(nested.name, "Bash");
        let flat: ResponsesTool = serde_json::from_value(json!({
            "type":"function",
            "name":"Read",
            "description":"read",
            "parameters":{"type":"object"}
        }))
        .unwrap();
        assert_eq!(flat.name, "Read");
    }

    #[test]
    fn test_parse_custom_tool_call_and_output() {
        let raw = json!([
            {"type":"message","role":"user","content":"run"},
            {"type":"custom_tool_call","call_id":"c1","name":"exec","input":"Get-Location"},
            {"type":"custom_tool_call_output","call_id":"c1","output":"C:\tmp"}
        ]);
        let msgs = parse_responses_input(&raw).unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[1].tool_calls[0].function.name, "exec");
        assert!(msgs[1].tool_calls[0].function.arguments.contains("Get-Location"));
        assert_eq!(msgs[2].tool_call_id.as_deref(), Some("c1"));
    }

    #[test]
    fn test_extract_additional_tools_from_input() {
        let raw = json!([
            {
                "type":"additional_tools",
                "role":"developer",
                "tools":[
                    {"type":"custom","name":"exec","description":"run js"},
                    {"type":"function","name":"wait","parameters":{"type":"object"}}
                ]
            },
            {"type":"message","role":"user","content":"hi"}
        ]);
        let tools = extract_additional_tools(&raw);
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].tool_type, "custom");
        assert_eq!(tools[0].name, "exec");
        assert_eq!(tools[1].name, "wait");
        // additional_tools 不应成为消息
        let msgs = parse_responses_input(&raw).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].text_content(), "hi");
    }
}
