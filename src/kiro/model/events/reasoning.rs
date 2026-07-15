//! 推理内容事件
//!
//! 处理 reasoningContentEvent 类型的事件。
//! - IDE GPT / 部分路径：`text` + `signature`
//! - CLI GPT：`redactedContent`（整段回灌 blob）
//! - Claude：常仅 `signature`，或 stream 后再组 reasoningText

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
    /// 推理文本（GPT IDE 可能为 `"..."` 占位；Claude 常缺省）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,

    /// 推理签名，后续请求 history 回灌时需要原样带回
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,

    /// CLI GPT 使用的 redact 形态（整段 blob，history 以 redactedContent 回灌）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redacted_content: Option<String>,

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
    /// 是否包含可回灌的签名 / redacted blob
    pub fn has_signature(&self) -> bool {
        self.effective_signature()
            .is_some_and(|s| !s.trim().is_empty())
    }

    /// 取可用于 history 回灌的签名：优先显式 signature，否则 redactedContent
    pub fn effective_signature(&self) -> Option<&str> {
        if let Some(sig) = self.signature.as_ref().map(|s| s.as_str()) {
            if !sig.trim().is_empty() {
                return Some(sig);
            }
        }
        self.redacted_content
            .as_ref()
            .map(|s| s.as_str())
            .filter(|s| !s.trim().is_empty())
    }

    /// 是否为 CLI GPT 的 redacted 形态
    pub fn is_redacted(&self) -> bool {
        self.redacted_content
            .as_ref()
            .is_some_and(|s| !s.trim().is_empty())
            && self
                .signature
                .as_ref()
                .map(|s| s.trim().is_empty())
                .unwrap_or(true)
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
        assert!(!event.is_redacted());
    }

    #[test]
    fn test_deserialize_cli_gpt_redacted() {
        let json = r#"{"redactedContent":"LktUUn5+blob"}"#;
        let event: ReasoningContentEvent = serde_json::from_str(json).unwrap();
        assert!(event.text.is_none());
        assert!(event.signature.is_none());
        assert_eq!(event.redacted_content.as_deref(), Some("LktUUn5+blob"));
        assert_eq!(event.effective_signature(), Some("LktUUn5+blob"));
        assert!(event.is_redacted());
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
        assert!(event.redacted_content.is_none());
        assert!(!event.has_signature());
    }
}
