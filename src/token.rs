//! Token 计算模块
//!
//! 提供文本 token 数量本地估算功能。
//!
//! # 计算规则
//! - 非西文字符：每个计 4 个字符单位
//! - 西文字符：每个计 1 个字符单位
//! - 4 个字符单位 = 1 token，再按区间加权

use crate::anthropic::types::{Message, SystemMessage, Tool};

/// 判断字符是否为非西文字符
fn is_non_western_char(c: char) -> bool {
    !matches!(c,
        // 基本 ASCII
        '\u{0000}'..='\u{007F}' |
        // 拉丁字母扩展-A
        '\u{0080}'..='\u{00FF}' |
        // 拉丁字母扩展-B
        '\u{0100}'..='\u{024F}' |
        // 拉丁字母扩展附加
        '\u{1E00}'..='\u{1EFF}' |
        // 拉丁字母扩展-C/D/E
        '\u{2C60}'..='\u{2C7F}' |
        '\u{A720}'..='\u{A7FF}' |
        '\u{AB30}'..='\u{AB6F}'
    )
}

/// 计算文本的 token 数量
pub fn count_tokens(text: &str) -> u64 {
    let char_units: f64 = text
        .chars()
        .map(|c| if is_non_western_char(c) { 4.0 } else { 1.0 })
        .sum();

    let tokens = char_units / 4.0;

    let acc_token = if tokens < 100.0 {
        tokens * 1.5
    } else if tokens < 200.0 {
        tokens * 1.3
    } else if tokens < 300.0 {
        tokens * 1.25
    } else if tokens < 800.0 {
        tokens * 1.2
    } else {
        tokens * 1.0
    } as u64;

    acc_token
}

/// 估算请求的输入 tokens（本地规则）
pub(crate) fn count_all_tokens(
    _model: String,
    system: Option<Vec<SystemMessage>>,
    messages: Vec<Message>,
    tools: Option<Vec<Tool>>,
) -> u64 {
    count_all_tokens_local(system, messages, tools)
}

fn count_all_tokens_local(
    system: Option<Vec<SystemMessage>>,
    messages: Vec<Message>,
    tools: Option<Vec<Tool>>,
) -> u64 {
    let mut total = 0;

    if let Some(ref system) = system {
        for msg in system {
            total += count_tokens(&msg.text);
        }
    }

    for msg in &messages {
        if let serde_json::Value::String(s) = &msg.content {
            total += count_tokens(s);
        } else if let serde_json::Value::Array(arr) = &msg.content {
            for item in arr {
                match item.get("type").and_then(|v| v.as_str()) {
                    Some("text") => {
                        if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                            total += count_tokens(text);
                        }
                    }
                    Some("thinking") => {
                        if let Some(text) = item.get("thinking").and_then(|v| v.as_str()) {
                            total += count_tokens(text);
                        }
                    }
                    Some("tool_use") => {
                        if let Some(input) = item.get("input") {
                            let s = serde_json::to_string(input).unwrap_or_default();
                            total += count_tokens(&s);
                        }
                    }
                    Some("tool_result") => match item.get("content") {
                        Some(serde_json::Value::String(s)) => {
                            total += count_tokens(s);
                        }
                        Some(serde_json::Value::Array(blocks)) => {
                            for block in blocks {
                                if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                                    total += count_tokens(text);
                                }
                            }
                        }
                        _ => {}
                    },
                    _ => {
                        let s = serde_json::to_string(item).unwrap_or_default();
                        total += count_tokens(&s);
                    }
                }
            }
        }
    }

    if let Some(ref tools) = tools {
        for tool in tools {
            total += count_tokens(&tool.name);
            total += count_tokens(&tool.description);
            let input_schema_json = serde_json::to_string(&tool.input_schema).unwrap_or_default();
            total += count_tokens(&input_schema_json);
        }
    }

    total.max(1)
}

/// 估算输出 tokens
pub(crate) fn estimate_output_tokens(content: &[serde_json::Value]) -> i32 {
    let mut total = 0;

    for block in content {
        if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
            total += count_tokens(text) as i32;
        }
        if block.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
            if let Some(input) = block.get("input") {
                let input_str = serde_json::to_string(input).unwrap_or_default();
                total += count_tokens(&input_str) as i32;
            }
        }
    }

    total.max(1)
}
