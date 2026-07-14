//! previous_response_id 历史展开

use super::input::parse_responses_input;
use super::store::ResponseStore;
use super::types::{OpenAIMessage, ResponseOutputItem, ResponsesObject, ToolCall};

/// 防止坏链无限循环
const MAX_RESPONSES_HISTORY_DEPTH: usize = 64;

/// 展开 previous_response 链为 OpenAI 消息列表（oldest → newest）
pub fn expand_previous_response_history(
    store: &ResponseStore,
    prev: &ResponsesObject,
) -> Vec<OpenAIMessage> {
    let chain = collect_ancestor_chain(store, prev);
    let mut messages = Vec::new();
    for node in chain {
        if let Some(instr) = node.instructions.as_ref() {
            if !instr.trim().is_empty() {
                messages.push(OpenAIMessage::system_text(instr));
            }
        }
        if let Some(input) = node.stored_input.as_ref() {
            if let Ok(prior) = parse_responses_input(input) {
                messages.extend(prior);
            }
        }
        messages.extend(output_to_messages(&node.output));
    }
    messages
}

fn collect_ancestor_chain<'a>(
    store: &ResponseStore,
    prev: &'a ResponsesObject,
) -> Vec<ResponsesObject> {
    let mut stack = vec![prev.clone()];
    let mut visited = std::collections::HashSet::new();
    visited.insert(prev.id.clone());

    let mut cursor_prev_id = prev.previous_response_id.clone();
    for _ in 0..MAX_RESPONSES_HISTORY_DEPTH {
        let Some(pid) = cursor_prev_id else { break };
        if !visited.insert(pid.clone()) {
            break;
        }
        match store.load(&pid) {
            Ok(ancestor) => {
                cursor_prev_id = ancestor.previous_response_id.clone();
                stack.push(ancestor);
            }
            Err(_) => break,
        }
    }
    stack.reverse(); // oldest first
    stack
}

pub fn output_to_messages(items: &[ResponseOutputItem]) -> Vec<OpenAIMessage> {
    let mut out = Vec::new();
    for item in items {
        match item.item_type.as_str() {
            "message" => {
                let text = join_text_parts(item);
                let role = item.role.as_deref().unwrap_or("assistant");
                if text.is_empty() && role == "assistant" {
                    continue;
                }
                out.push(OpenAIMessage {
                    role: role.to_string(),
                    content: serde_json::Value::String(text),
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                });
            }
            "function_call" | "custom_tool_call" | "tool_search_call" => {
                let call_id = item
                    .call_id
                    .clone()
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| item.id.clone());
                let name = item.name.clone().unwrap_or_default();
                let args = if item.item_type == "custom_tool_call" {
                    let input = item
                        .input
                        .clone()
                        .or_else(|| item.arguments.clone())
                        .unwrap_or_default();
                    serde_json::json!({"input": input}).to_string()
                } else {
                    item.arguments.clone().unwrap_or_else(|| "{}".to_string())
                };
                // 合并连续 function/custom tool calls
                if let Some(last) = out.last_mut() {
                    if last.role == "assistant"
                        && !last.tool_calls.is_empty()
                        && last.text_content().trim().is_empty()
                    {
                        last.tool_calls
                            .push(ToolCall::function(call_id, name, args));
                        continue;
                    }
                }
                out.push(OpenAIMessage::assistant_tool_calls(vec![ToolCall::function(
                    call_id, name, args,
                )]));
            }
            _ => {}
        }
    }
    out
}

fn join_text_parts(item: &ResponseOutputItem) -> String {
    let mut out = String::new();
    for p in &item.content {
        if matches!(p.part_type.as_str(), "output_text" | "text" | "input_text") {
            if let Some(t) = &p.text {
                out.push_str(t);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openai::store::{generate_output_item_id, generate_response_id, ResponseStore};
    use crate::openai::types::{ResponseContentPart, ResponsesUsage};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_store() -> (ResponseStore, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "kiro-rs-hist-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        (ResponseStore::new(&dir), dir)
    }

    fn msg_output(text: &str) -> ResponseOutputItem {
        ResponseOutputItem {
            id: generate_output_item_id("msg"),
            item_type: "message".to_string(),
            role: Some("assistant".to_string()),
            status: Some("completed".to_string()),
            content: vec![ResponseContentPart {
                part_type: "output_text".to_string(),
                text: Some(text.to_string()),
            }],
            call_id: None,
            name: None,
            arguments: None,
            input: None,
        }
    }

    #[test]
    fn test_expand_single_previous() {
        let (store, dir) = tmp_store();
        let id = generate_response_id();
        let prev = ResponsesObject {
            id: id.clone(),
            object: "response".to_string(),
            created_at: 1,
            status: "completed".to_string(),
            model: "gpt-5.6-luna".to_string(),
            output: vec![msg_output("assistant-1")],
            usage: ResponsesUsage::default(),
            previous_response_id: None,
            metadata: None,
            error: None,
            instructions: Some("sys-a".to_string()),
            stored_input: Some(serde_json::json!("user-1")),
            stored_at: None,
        };
        store.save(&prev).unwrap();
        let loaded = store.load(&id).unwrap();
        let msgs = expand_previous_response_history(&store, &loaded);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, "system");
        assert_eq!(msgs[0].text_content(), "sys-a");
        assert_eq!(msgs[1].role, "user");
        assert_eq!(msgs[1].text_content(), "user-1");
        assert_eq!(msgs[2].role, "assistant");
        assert_eq!(msgs[2].text_content(), "assistant-1");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_expand_full_chain() {
        let (store, dir) = tmp_store();
        let a_id = "resp_a_chain";
        let b_id = "resp_b_chain";
        let a = ResponsesObject {
            id: a_id.to_string(),
            object: "response".to_string(),
            created_at: 1,
            status: "completed".to_string(),
            model: "m".to_string(),
            output: vec![msg_output("A-out")],
            usage: ResponsesUsage::default(),
            previous_response_id: None,
            metadata: None,
            error: None,
            instructions: Some("sys-a".to_string()),
            stored_input: Some(serde_json::json!("A-in")),
            stored_at: None,
        };
        let b = ResponsesObject {
            id: b_id.to_string(),
            object: "response".to_string(),
            created_at: 2,
            status: "completed".to_string(),
            model: "m".to_string(),
            output: vec![msg_output("B-out")],
            usage: ResponsesUsage::default(),
            previous_response_id: Some(a_id.to_string()),
            metadata: None,
            error: None,
            instructions: None,
            stored_input: Some(serde_json::json!("B-in")),
            stored_at: None,
        };
        store.save(&a).unwrap();
        store.save(&b).unwrap();
        let loaded_b = store.load(b_id).unwrap();
        let msgs = expand_previous_response_history(&store, &loaded_b);
        // sys-a, A-in, A-out, B-in, B-out
        assert_eq!(msgs.len(), 5);
        assert_eq!(msgs[0].text_content(), "sys-a");
        assert_eq!(msgs[1].text_content(), "A-in");
        assert_eq!(msgs[2].text_content(), "A-out");
        assert_eq!(msgs[3].text_content(), "B-in");
        assert_eq!(msgs[4].text_content(), "B-out");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_output_function_call_to_messages() {
        let items = vec![
            ResponseOutputItem {
                id: "fc_1".to_string(),
                item_type: "function_call".to_string(),
                role: None,
                status: Some("completed".to_string()),
                content: vec![],
                call_id: Some("call_1".to_string()),
                name: Some("Bash".to_string()),
                arguments: Some(r#"{"command":"ls"}"#.to_string()),
                input: None,
            },
            ResponseOutputItem {
                id: "fc_2".to_string(),
                item_type: "function_call".to_string(),
                role: None,
                status: Some("completed".to_string()),
                content: vec![],
                call_id: Some("call_2".to_string()),
                name: Some("Glob".to_string()),
                arguments: Some(r#"{"pattern":"*"}"#.to_string()),
                input: None,
            },
        ];
        let msgs = output_to_messages(&items);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].tool_calls.len(), 2);
    }
}
