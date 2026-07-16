//! OpenAI Responses API 类型

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// POST /v1/responses 请求体
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponsesRequest {
    #[serde(default)]
    pub model: String,
    /// string | array | object
    #[serde(default)]
    pub input: Value,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub tools: Option<Vec<ResponsesTool>>,
    #[serde(default)]
    pub tool_choice: Option<Value>,
    #[serde(default)]
    pub previous_response_id: Option<String>,
    /// 默认 true
    #[serde(default)]
    pub store: Option<bool>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub max_output_tokens: Option<i32>,
    #[serde(default)]
    pub metadata: Option<std::collections::HashMap<String, String>>,
}

impl ResponsesRequest {
    pub fn should_store(&self) -> bool {
        self.store.unwrap_or(true)
    }
}

/// Responses / Chat Completions 兼容的 tool 定义
///
/// 支持：
/// - Chat: `{ "type":"function", "function":{ "name", "description", "parameters" } }`
/// - Responses function: `{ "type":"function", "name", "description", "parameters" }`
/// - Codex custom: `{ "type":"custom", "name":"exec", ... }` 或字符串简写 `"exec"`
/// - namespace: `{ "type":"namespace", "name":"ns", "tools":[...] }`
/// - 服务端工具: `web_search` / `tool_search` / `image_generation`（转换层按需保留/丢弃）
#[derive(Debug, Clone, Serialize)]
pub struct ResponsesTool {
    #[serde(rename = "type", default = "default_tool_type")]
    pub tool_type: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub parameters: Option<Value>,
    /// namespace 子工具（Codex / OpenAI Responses）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ResponsesTool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<ResponsesTool>,
}

fn default_tool_type() -> String {
    "function".to_string()
}

impl<'de> Deserialize<'de> for ResponsesTool {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // Codex 允许 tools 数组里写字符串简写（视为 custom 工具名）
        let value = Value::deserialize(deserializer)?;
        if let Value::String(name) = value {
            return Ok(ResponsesTool {
                tool_type: "custom".to_string(),
                name,
                description: None,
                parameters: None,
                tools: Vec::new(),
                children: Vec::new(),
            });
        }

        #[derive(Deserialize)]
        struct Raw {
            #[serde(rename = "type")]
            tool_type: Option<String>,
            name: Option<String>,
            description: Option<String>,
            parameters: Option<Value>,
            function: Option<RawFunction>,
            #[serde(default)]
            tools: Vec<ResponsesTool>,
            #[serde(default)]
            children: Vec<ResponsesTool>,
        }
        #[derive(Deserialize)]
        struct RawFunction {
            name: Option<String>,
            description: Option<String>,
            parameters: Option<Value>,
        }

        let raw: Raw = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        let mut name = raw.name.unwrap_or_default();
        let mut description = raw.description;
        let mut parameters = raw.parameters;
        if let Some(func) = raw.function {
            if name.is_empty() {
                name = func.name.unwrap_or_default();
            }
            if description.is_none() {
                description = func.description;
            }
            if parameters.is_none() {
                parameters = func.parameters;
            }
        }
        let tool_type = raw.tool_type.unwrap_or_else(|| {
            if !name.is_empty() {
                "custom".to_string()
            } else {
                default_tool_type()
            }
        });
        Ok(ResponsesTool {
            tool_type,
            name,
            description,
            parameters,
            tools: raw.tools,
            children: raw.children,
        })
    }
}

/// 中间层 OpenAI 消息（用于 history 展开 / 转 Anthropic）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenAIMessage {
    pub role: String,
    #[serde(default)]
    pub content: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl OpenAIMessage {
    pub fn user_text(text: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: Value::String(text.into()),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn system_text(text: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: Value::String(text.into()),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn assistant_text(text: impl Into<String>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: Value::String(text.into()),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn assistant_tool_calls(tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: Value::String(String::new()),
            tool_calls,
            tool_call_id: None,
        }
    }

    pub fn tool_result(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".to_string(),
            content: Value::String(content.into()),
            tool_calls: Vec::new(),
            tool_call_id: Some(call_id.into()),
        }
    }

    pub fn text_content(&self) -> String {
        match &self.content {
            Value::String(s) => s.clone(),
            Value::Array(parts) => {
                let mut out = String::new();
                for p in parts {
                    if let Some(obj) = p.as_object() {
                        let t = obj
                            .get("type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if matches!(t, "text" | "input_text" | "output_text") {
                            if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                                out.push_str(text);
                            }
                        }
                    }
                }
                out
            }
            Value::Object(obj) => obj
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            _ => String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type", default = "default_tool_type")]
    pub call_type: String,
    pub function: ToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCallFunction {
    pub name: String,
    #[serde(default)]
    pub arguments: String,
}

impl ToolCall {
    pub fn function(id: impl Into<String>, name: impl Into<String>, arguments: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            call_type: "function".to_string(),
            function: ToolCallFunction {
                name: name.into(),
                arguments: arguments.into(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesObject {
    pub id: String,
    pub object: String,
    pub created_at: i64,
    pub status: String,
    pub model: String,
    pub output: Vec<ResponseOutputItem>,
    pub usage: ResponsesUsage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<std::collections::HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ResponsesError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// 仅持久化使用，不对外序列化
    #[serde(default, skip_serializing)]
    pub stored_input: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stored_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseOutputItem {
    pub id: String,
    #[serde(rename = "type")]
    pub item_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<ResponseContentPart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    /// custom_tool_call freeform input（Codex 路由 custom 工具用）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseContentPart {
    #[serde(rename = "type")]
    pub part_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponsesUsage {
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub total_tokens: i32,
    /// OpenAI Responses：输入中命中缓存的 token 数（模拟 prompt cache）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens_details: Option<ResponsesInputTokensDetails>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponsesInputTokensDetails {
    #[serde(default)]
    pub cached_tokens: i32,
}

impl ResponsesUsage {
    pub fn with_cache(input_tokens: i32, output_tokens: i32, cached_tokens: i32) -> Self {
        let input_tokens = input_tokens.max(0);
        let output_tokens = output_tokens.max(0);
        let cached_tokens = cached_tokens.max(0).min(input_tokens);
        Self {
            input_tokens,
            output_tokens,
            total_tokens: input_tokens + output_tokens,
            input_tokens_details: Some(ResponsesInputTokensDetails { cached_tokens }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesError {
    #[serde(rename = "type")]
    pub error_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub message: String,
}

/// OpenAI 风格错误响应
#[derive(Debug, Serialize)]
pub struct OpenAIErrorResponse {
    pub error: OpenAIErrorDetail,
}

#[derive(Debug, Serialize)]
pub struct OpenAIErrorDetail {
    #[serde(rename = "type")]
    pub error_type: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

impl OpenAIErrorResponse {
    pub fn new(error_type: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error: OpenAIErrorDetail {
                error_type: error_type.into(),
                message: message.into(),
                code: None,
            },
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_responses_usage_with_cache_serializes_cached_tokens() {
        let u = ResponsesUsage::with_cache(1000, 20, 800);
        let v = serde_json::to_value(&u).unwrap();
        assert_eq!(v["input_tokens"], 1000);
        assert_eq!(v["output_tokens"], 20);
        assert_eq!(v["total_tokens"], 1020);
        assert_eq!(v["input_tokens_details"]["cached_tokens"], 800);

        // clamp cached to input
        let u2 = ResponsesUsage::with_cache(100, 1, 999);
        let v2 = serde_json::to_value(&u2).unwrap();
        assert_eq!(v2["input_tokens_details"]["cached_tokens"], 100);
    }
}
