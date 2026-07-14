//! 推理内容事件
//!
//! 处理 reasoningContentEvent 类型的事件。
//! GPT 5.6 会返回 text + signature；Claude 常仅返回 signature。

use serde::{Deserialize, Serialize};

use crate::kiro::parser::error::ParseResult;
use crate::kiro::parser::frame::Frame;

use super::base::EventPayload;

/// 推理内容事件
///
/// 对应 Kiro 上游 `reasoningContentEvent`。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningContentEvent {
    /// 推理文本（GPT 可能为 `"..."` 占位；Claude 常缺省）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,

    /// 推理签名，后续请求 history 回灌时需要原样带回
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,

    /// 捕获其他未使用字段，保证兼容性
    #[serde(flatten)]
    #[serde(skip_serializing)]
    #[allow(dead_code)]
    extra: serde_json::Value,
}

impl EventPayload for ReasoningContentEvent {
    fn from_frame(frame: &Frame) -> ParseResult<Self> {
        frame.payload_as_json()
    }
}

impl ReasoningContentEvent {
    /// 是否包含可回灌的签名
    pub fn has_signature(&self) -> bool {
        self.signature
            .as_ref()
            .is_some_and(|s| !s.trim().is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_gpt_style() {
        let json = r#"{"text":"...","signature":".KTR~~abc"}"#;
        let event: ReasoningContentEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.text.as_deref(), Some("..."));
        assert_eq!(event.signature.as_deref(), Some(".KTR~~abc"));
        assert!(event.has_signature());
    }

    #[test]
    fn test_deserialize_claude_style_signature_only() {
        let json = r#"{"signature":"EoYDCnIIDxABGAIqQGcm"}"#;
        let event: ReasoningContentEvent = serde_json::from_str(json).unwrap();
        assert!(event.text.is_none());
        assert_eq!(event.signature.as_deref(), Some("EoYDCnIIDxABGAIqQGcm"));
        assert!(event.has_signature());
    }

    #[test]
    fn test_deserialize_empty() {
        let json = r#"{}"#;
        let event: ReasoningContentEvent = serde_json::from_str(json).unwrap();
        assert!(event.text.is_none());
        assert!(event.signature.is_none());
        assert!(!event.has_signature());
    }
}
