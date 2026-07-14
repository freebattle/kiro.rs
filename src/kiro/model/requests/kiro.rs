//! Kiro 请求类型定义
//!
//! 定义 Kiro API 的主请求结构

use serde::{Deserialize, Serialize};

use super::conversation::ConversationState;

/// Kiro API 请求
///
/// 用于构建发送给 Kiro API 的请求
///
/// # 示例
///
/// ```rust
/// use kiro_rs::kiro::model::requests::{
///     KiroRequest, ConversationState, CurrentMessage, UserInputMessage, Tool
/// };
///
/// // 创建简单请求
/// let state = ConversationState::new("conv-123")
///     .with_agent_task_type("vibe")
///     .with_current_message(CurrentMessage::new(
///         UserInputMessage::new("Hello", "claude-3-5-sonnet")
///     ));
///
/// let request = KiroRequest::new(state);
/// let json = request.to_json().unwrap();
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KiroRequest {
    /// 对话状态
    pub conversation_state: ConversationState,
    /// Profile ARN（可选；IDE 端点会在发送前注入）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_arn: Option<String>,
    /// Agent 模式（官方 Kiro 1.0.138+ 根字段，与 conversationState.agentTaskType 对齐）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_mode: Option<String>,
    /// 额外模型请求字段（GPT 5.6 需要 reasoning.effort）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_model_request_fields: Option<serde_json::Value>,
}

impl KiroRequest {
    pub fn new(conversation_state: ConversationState) -> Self {
        Self {
            conversation_state,
            profile_arn: None,
            agent_mode: None,
            additional_model_request_fields: None,
        }
    }

    pub fn with_agent_mode(mut self, mode: impl Into<String>) -> Self {
        self.agent_mode = Some(mode.into());
        self
    }

    pub fn with_additional_model_request_fields(mut self, fields: serde_json::Value) -> Self {
        self.additional_model_request_fields = Some(fields);
        self
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kiro::model::requests::conversation::{CurrentMessage, UserInputMessage};

    #[test]
    fn test_kiro_request_deserialize() {
        let json = r#"{
            "conversationState": {
                "conversationId": "conv-456",
                "currentMessage": {
                    "userInputMessage": {
                        "content": "Test message",
                        "modelId": "claude-3-5-sonnet",
                        "userInputMessageContext": {}
                    }
                }
            }
        }"#;

        let request: KiroRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.conversation_state.conversation_id, "conv-456");
        assert_eq!(
            request
                .conversation_state
                .current_message
                .user_input_message
                .content,
            "Test message"
        );
        assert!(request.agent_mode.is_none());
        assert!(request.additional_model_request_fields.is_none());
    }

    #[test]
    fn test_kiro_request_serializes_gpt_fields() {
        let state = ConversationState::new("conv-1").with_current_message(CurrentMessage::new(
            UserInputMessage::new("hi", "gpt-5.6-luna"),
        ));
        let request = KiroRequest::new(state)
            .with_agent_mode("vibe")
            .with_additional_model_request_fields(serde_json::json!({
                "reasoning": { "effort": "high" }
            }));
        let v = serde_json::to_value(&request).unwrap();
        assert_eq!(v["agentMode"], "vibe");
        assert_eq!(v["additionalModelRequestFields"]["reasoning"]["effort"], "high");
        assert!(v.get("profileArn").is_none());
    }
}
