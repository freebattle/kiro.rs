//! Kiro CLI 端点
//!
//! 对齐官方 Kiro CLI / Amazon Q for CLI（抓包 2026-07-14，含思考强度）：
//! - API: `https://runtime.{api_region}.kiro.dev`
//! - `x-amz-target: AmazonCodeWhispererStreamingService.GenerateAssistantResponse`
//! - Content-Type: `application/x-amz-json-1.0`
//! - User-Agent: aws-sdk-rust + `app/AmazonQ-For-CLI`
//! - 请求体：
//!   - `conversationState` + `profileArn`
//!   - **可带** `additionalModelRequestFields`（选思考强度时）：
//!     - Claude: `{"output_config":{"effort":"high"}}`
//!     - GPT: `{"reasoning":{"effort":"medium"}}`
//!   - **不发** 根级 `agentMode`
//!   - **不发** `conversationState.agentContinuationId`
//!   - `origin` = `KIRO_CLI`
//!   - history 用户消息通常 **不带** `modelId`
//!   - GPT history 的 reasoning 用 `redactedContent`（不是 reasoningText）
//!   - 带 `x-amzn-codewhisperer-optout: false`，不带 `TokenType`

use reqwest::RequestBuilder;
use uuid::Uuid;

use crate::kiro::user_agent;

use super::{KiroEndpoint, RequestContext};

/// Kiro CLI 端点名称（对应 `defaultEndpoint` / `credentials.endpoint` = `"cli"`）
pub const CLI_ENDPOINT_NAME: &str = "cli";

/// GenerateAssistantResponse 的 x-amz-target（CLI / Amazon Q for CLI）
const CLI_GENERATE_TARGET: &str = "AmazonCodeWhispererStreamingService.GenerateAssistantResponse";

/// Kiro CLI 端点
pub struct CliEndpoint;

impl CliEndpoint {
    pub fn new() -> Self {
        Self
    }

    fn api_region<'a>(&self, ctx: &'a RequestContext<'_>) -> &'a str {
        ctx.credentials.effective_api_region()
    }

    fn host(&self, ctx: &RequestContext<'_>) -> String {
        format!("runtime.{}.kiro.dev", self.api_region(ctx))
    }

    fn x_amz_user_agent(&self, ctx: &RequestContext<'_>) -> String {
        user_agent::cli_streaming_x_amz_user_agent()
    }

    fn user_agent(&self, ctx: &RequestContext<'_>) -> String {
        user_agent::cli_streaming_user_agent()
    }
}

impl Default for CliEndpoint {
    fn default() -> Self {
        Self::new()
    }
}

impl KiroEndpoint for CliEndpoint {
    fn name(&self) -> &'static str {
        CLI_ENDPOINT_NAME
    }

    fn api_url(&self, ctx: &RequestContext<'_>) -> String {
        format!("https://runtime.{}.kiro.dev", self.api_region(ctx))
    }

    fn mcp_url(&self, ctx: &RequestContext<'_>) -> String {
        // CLI 主生成路径未使用 MCP；保留同 host 约定，避免凭据误配时崩溃
        format!("https://runtime.{}.kiro.dev/mcp", self.api_region(ctx))
    }

    fn api_content_type(&self) -> &'static str {
        "application/x-amz-json-1.0"
    }

    fn decorate_api(&self, req: RequestBuilder, ctx: &RequestContext<'_>) -> RequestBuilder {
        req.header("x-amz-target", CLI_GENERATE_TARGET)
            .header("x-amz-user-agent", self.x_amz_user_agent(ctx))
            .header("user-agent", self.user_agent(ctx))
            .header("host", self.host(ctx))
            .header("amz-sdk-invocation-id", Uuid::new_v4().to_string())
            .header("amz-sdk-request", "attempt=1; max=3")
            .header("x-amzn-codewhisperer-optout", "false")
            .header("Authorization", format!("Bearer {}", ctx.token))
    }

    fn decorate_mcp(&self, req: RequestBuilder, ctx: &RequestContext<'_>) -> RequestBuilder {
        let mut req = req
            .header("x-amz-user-agent", self.x_amz_user_agent(ctx))
            .header("user-agent", self.user_agent(ctx))
            .header("host", self.host(ctx))
            .header("amz-sdk-invocation-id", Uuid::new_v4().to_string())
            .header("amz-sdk-request", "attempt=1; max=3")
            .header("x-amzn-codewhisperer-optout", "false")
            .header("Authorization", format!("Bearer {}", ctx.token));

        if let Some(ref arn) = ctx.credentials.profile_arn {
            req = req.header("x-amzn-kiro-profile-arn", arn);
        }
        req
    }

    fn transform_api_body(&self, body: &str, ctx: &RequestContext<'_>) -> String {
        transform_cli_api_body(body, &ctx.credentials.profile_arn)
    }
}

/// 将 converter 产出的 IDE 风格 body 收敛为 CLI 风格：
/// - 注入 profileArn
/// - 去掉 agentMode / agentContinuationId（CLI 不发）
/// - **保留** additionalModelRequestFields（CLI 选思考强度时会发）
/// - origin 统一为 KIRO_CLI
/// - history 用户消息去掉 modelId
/// - GPT history reasoningText → redactedContent
fn transform_cli_api_body(request_body: &str, profile_arn: &Option<String>) -> String {
    let Ok(mut json) = serde_json::from_str::<serde_json::Value>(request_body) else {
        return request_body.to_string();
    };

    if let Some(obj) = json.as_object_mut() {
        // CLI 永不发根级 agentMode；effort 字段保留
        obj.remove("agentMode");

        if let Some(arn) = profile_arn {
            obj.insert(
                "profileArn".to_string(),
                serde_json::Value::String(arn.clone()),
            );
        }

        if let Some(cs) = obj
            .get_mut("conversationState")
            .and_then(|v| v.as_object_mut())
        {
            cs.remove("agentContinuationId");
            rewrite_conversation_state_for_cli(cs);
        }
    }

    serde_json::to_string(&json).unwrap_or_else(|_| request_body.to_string())
}

fn rewrite_conversation_state_for_cli(cs: &mut serde_json::Map<String, serde_json::Value>) {
    if let Some(current) = cs
        .get_mut("currentMessage")
        .and_then(|v| v.as_object_mut())
    {
        if let Some(uim) = current
            .get_mut("userInputMessage")
            .and_then(|v| v.as_object_mut())
        {
            uim.insert(
                "origin".to_string(),
                serde_json::Value::String("KIRO_CLI".to_string()),
            );
        }
    }

    if let Some(history) = cs.get_mut("history").and_then(|v| v.as_array_mut()) {
        for item in history {
            if let Some(uim) = item
                .get_mut("userInputMessage")
                .and_then(|v| v.as_object_mut())
            {
                uim.insert(
                    "origin".to_string(),
                    serde_json::Value::String("KIRO_CLI".to_string()),
                );
                // CLI history 用户消息不带 modelId
                uim.remove("modelId");
            }

            if let Some(arm) = item
                .get_mut("assistantResponseMessage")
                .and_then(|v| v.as_object_mut())
            {
                rewrite_assistant_reasoning_for_cli(arm);
            }
        }
    }
}

/// GPT CLI history 使用 `reasoningContent.redactedContent`；
/// converter 默认产出 `reasoningText{text,signature}`（IDE 风格）。
/// 当 text 为占位（"..." / 空 / Completed. / Done.）时，收敛为 redactedContent。
fn rewrite_assistant_reasoning_for_cli(arm: &mut serde_json::Map<String, serde_json::Value>) {
    let Some(rc) = arm.get_mut("reasoningContent") else {
        return;
    };
    // 已是 redacted 形态
    if rc.get("redactedContent").is_some() {
        return;
    }

    let Some(rt) = rc.get("reasoningText") else {
        return;
    };
    let text = rt.get("text").and_then(|v| v.as_str()).unwrap_or("");
    let sig = rt
        .get("signature")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if sig.is_empty() {
        return;
    }

    let placeholder = matches!(text.trim(), "" | "..." | "Completed." | "Done.");
    // 长 signature（CLI GPT redacted blob / 或可回灌签名）且文本为占位 → redactedContent
    // Claude 的 reasoningText（真实 thinking 文本 + 签名）保持不变
    if placeholder {
        *rc = serde_json::json!({ "redactedContent": sig });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_cli_endpoint_name() {
        assert_eq!(CliEndpoint::new().name(), "cli");
        assert_eq!(
            CliEndpoint::new().api_content_type(),
            "application/x-amz-json-1.0"
        );
    }

    #[test]
    fn test_transform_keeps_effort_strips_agent_mode_sets_cli_origin() {
        let body = json!({
            "conversationState": {
                "agentContinuationId": "cont-1",
                "agentTaskType": "vibe",
                "chatTriggerType": "MANUAL",
                "conversationId": "c1",
                "currentMessage": {
                    "userInputMessage": {
                        "content": "hi",
                        "modelId": "claude-opus-4.8",
                        "origin": "AI_EDITOR"
                    }
                },
                "history": [
                    {
                        "userInputMessage": {
                            "content": "sys",
                            "modelId": "claude-opus-4.8",
                            "origin": "AI_EDITOR"
                        }
                    },
                    {
                        "assistantResponseMessage": {
                            "content": "ok",
                            "reasoningContent": {
                                "reasoningText": {
                                    "text": "plan",
                                    "signature": "EoYDsig"
                                }
                            }
                        }
                    },
                    {
                        "assistantResponseMessage": {
                            "content": "",
                            "toolUses": [{"toolUseId":"call_1","name":"read","input":{}}],
                            "reasoningContent": {
                                "reasoningText": {
                                    "text": "...",
                                    "signature": "LktUUn5+REDACTED_BLOB"
                                }
                            }
                        }
                    }
                ]
            },
            "agentMode": "vibe",
            "additionalModelRequestFields": {
                "output_config": { "effort": "high" }
            }
        })
        .to_string();

        let arn = Some("arn:aws:codewhisperer:us-east-1:123:profile/ABC".to_string());
        let out = transform_cli_api_body(&body, &arn);
        let json: serde_json::Value = serde_json::from_str(&out).unwrap();

        // CLI 专有：无 agentMode / 无 continuation；保留 effort
        assert!(json.get("agentMode").is_none());
        assert_eq!(
            json["additionalModelRequestFields"]["output_config"]["effort"],
            "high"
        );
        assert_eq!(
            json["profileArn"],
            "arn:aws:codewhisperer:us-east-1:123:profile/ABC"
        );
        assert!(json["conversationState"]
            .get("agentContinuationId")
            .is_none());
        assert_eq!(json["conversationState"]["agentTaskType"], "vibe");
        assert_eq!(
            json["conversationState"]["currentMessage"]["userInputMessage"]["origin"],
            "KIRO_CLI"
        );
        // history 用户：origin 改写 + 去掉 modelId
        assert_eq!(
            json["conversationState"]["history"][0]["userInputMessage"]["origin"],
            "KIRO_CLI"
        );
        assert!(json["conversationState"]["history"][0]["userInputMessage"]
            .get("modelId")
            .is_none());
        // Claude 真实 thinking 文本：保持 reasoningText
        assert_eq!(
            json["conversationState"]["history"][1]["assistantResponseMessage"]["reasoningContent"]
                ["reasoningText"]["text"],
            "plan"
        );
        // GPT 占位 text + signature → redactedContent
        assert_eq!(
            json["conversationState"]["history"][2]["assistantResponseMessage"]["reasoningContent"]
                ["redactedContent"],
            "LktUUn5+REDACTED_BLOB"
        );
        assert!(json["conversationState"]["history"][2]["assistantResponseMessage"]
            ["reasoningContent"]
            .get("reasoningText")
            .is_none());
    }

    #[test]
    fn test_transform_gpt_reasoning_effort_kept() {
        let body = json!({
            "conversationState": {
                "agentTaskType": "vibe",
                "currentMessage": {
                    "userInputMessage": {
                        "content": "hi",
                        "modelId": "gpt-5.6-sol",
                        "origin": "AI_EDITOR"
                    }
                }
            },
            "agentMode": "vibe",
            "additionalModelRequestFields": {
                "reasoning": { "effort": "medium" }
            }
        })
        .to_string();
        let out = transform_cli_api_body(&body, &None);
        let json: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert!(json.get("agentMode").is_none());
        assert_eq!(
            json["additionalModelRequestFields"]["reasoning"]["effort"],
            "medium"
        );
        assert_eq!(
            json["conversationState"]["currentMessage"]["userInputMessage"]["origin"],
            "KIRO_CLI"
        );
    }

    #[test]
    fn test_transform_without_profile_arn() {
        let body = r#"{"conversationState":{"currentMessage":{"userInputMessage":{"origin":"AI_EDITOR"}}},"agentMode":"vibe"}"#;
        let out = transform_cli_api_body(body, &None);
        let json: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert!(json.get("profileArn").is_none());
        assert!(json.get("agentMode").is_none());
        assert_eq!(
            json["conversationState"]["currentMessage"]["userInputMessage"]["origin"],
            "KIRO_CLI"
        );
    }

    #[test]
    fn test_transform_invalid_json_passthrough() {
        let body = "not-json";
        let out = transform_cli_api_body(body, &Some("arn".into()));
        assert_eq!(out, "not-json");
    }
}
