//! Kiro IDE 端点
//!
//! 对应 Kiro IDE 1.2.56 客户端当前使用的 Kiro Runtime 端点：
//! - API: `https://runtime.{api_region}.kiro.dev` + `x-amz-target: KiroRuntimeService.GenerateAssistantResponse`
//! - MCP: `https://runtime.{api_region}.kiro.dev` + `x-amz-target: KiroRuntimeService.InvokeMCP`
//!
//! 请求头使用 aws-sdk-js User-Agent 标识（api/kiroruntime#1.0.0）。
//! API 与 MCP 请求体都会在根对象上注入 `profileArn`。

use reqwest::RequestBuilder;
use uuid::Uuid;

use crate::kiro::user_agent;

use super::{KiroEndpoint, RequestContext};

/// Kiro IDE 端点名称
pub const IDE_ENDPOINT_NAME: &str = "ide";

/// Kiro IDE 端点
pub struct IdeEndpoint;

impl IdeEndpoint {
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
        user_agent::runtime_streaming_x_amz_user_agent(ctx.machine_id)
    }

    fn user_agent(&self, ctx: &RequestContext<'_>) -> String {
        user_agent::runtime_streaming_user_agent(ctx.machine_id)
    }

    /// 官方 Kiro 1.2.56 使用 `TokenType` 头：
    /// - SSO 登录: SSO_OIDC
    /// - API Key: API_KEY
    fn token_type(&self, ctx: &RequestContext<'_>) -> &'static str {
        if ctx.credentials.is_api_key_credential() {
            "API_KEY"
        } else {
            "SSO_OIDC"
        }
    }
}

impl Default for IdeEndpoint {
    fn default() -> Self {
        Self::new()
    }
}

impl KiroEndpoint for IdeEndpoint {
    fn name(&self) -> &'static str {
        IDE_ENDPOINT_NAME
    }

    fn api_url(&self, ctx: &RequestContext<'_>) -> String {
        // 1.0.138+ / 1.0.437：根路径 + x-amz-target，不再走 /generateAssistantResponse
        format!("https://runtime.{}.kiro.dev", self.api_region(ctx))
    }

    fn mcp_url(&self, ctx: &RequestContext<'_>) -> String {
        // 1.2.56：MCP 改走根路径 + x-amz-target InvokeMCP，不再请求 /mcp
        format!("https://runtime.{}.kiro.dev", self.api_region(ctx))
    }

    fn api_content_type(&self) -> &'static str {
        "application/x-amz-json-1.0"
    }

    fn mcp_content_type(&self) -> &'static str {
        "application/x-amz-json-1.0"
    }

    fn decorate_api(&self, req: RequestBuilder, ctx: &RequestContext<'_>) -> RequestBuilder {
        // agentMode 已在请求体根字段发送，官方 1.0.138+ 不再带 x-amzn-kiro-agent-mode 头
        req.header(
            "x-amz-target",
            "KiroRuntimeService.GenerateAssistantResponse",
        )
        .header("x-amzn-codewhisperer-optout", "true")
        .header("x-amzn-kiro-client-attribution", "kiro-ide")
        .header("x-amz-user-agent", self.x_amz_user_agent(ctx))
        .header("user-agent", self.user_agent(ctx))
        .header("host", self.host(ctx))
        .header("amz-sdk-invocation-id", Uuid::new_v4().to_string())
        .header("amz-sdk-request", "attempt=1; max=3")
        .header("x-kiro-attempt", "1;max=3")
        .header("TokenType", self.token_type(ctx))
        .header("Authorization", format!("Bearer {}", ctx.token))
    }

    fn decorate_mcp(&self, req: RequestBuilder, ctx: &RequestContext<'_>) -> RequestBuilder {
        // 官方 1.2.56 InvokeMCP：与 GenerateAssistantResponse 同 runtime UA（含 KAS），
        // 总带 TokenType；profileArn 在请求体内，不再发 x-amzn-kiro-profile-arn 头
        req.header("x-amz-target", "KiroRuntimeService.InvokeMCP")
            .header("TokenType", self.token_type(ctx))
            .header("x-amz-user-agent", self.x_amz_user_agent(ctx))
            .header("user-agent", self.user_agent(ctx))
            .header("host", self.host(ctx))
            .header("amz-sdk-invocation-id", Uuid::new_v4().to_string())
            .header("amz-sdk-request", "attempt=1; max=3")
            .header("Authorization", format!("Bearer {}", ctx.token))
    }

    fn transform_api_body(&self, body: &str, ctx: &RequestContext<'_>) -> String {
        inject_profile_arn(body, &ctx.credentials.profile_arn)
    }

    fn transform_mcp_body(&self, body: &str, ctx: &RequestContext<'_>) -> String {
        inject_profile_arn(body, &ctx.credentials.profile_arn)
    }
}

/// 将 profile_arn 注入到请求体 JSON 根对象
fn inject_profile_arn(request_body: &str, profile_arn: &Option<String>) -> String {
    if let Some(arn) = profile_arn {
        if let Ok(mut json) = serde_json::from_str::<serde_json::Value>(request_body) {
            json["profileArn"] = serde_json::Value::String(arn.clone());
            if let Ok(body) = serde_json::to_string(&json) {
                return body;
            }
        }
    }
    request_body.to_string()
}

#[cfg(test)]
mod tests {
    use super::{IdeEndpoint, inject_profile_arn};
    use crate::kiro::endpoint::{KiroEndpoint, RequestContext};
    use crate::kiro::model::credentials::KiroCredentials;
    use crate::model::config::{Config, KAS_VERSION, KIRO_VERSION};
    use serde_json::Value;

    /// 对齐官方 1.2.56 抓包：web_search 走根路径 KiroRuntimeService.InvokeMCP
    #[test]
    fn test_mcp_request_matches_invoke_mcp_capture() {
        let mut credentials = KiroCredentials::default();
        credentials.profile_arn =
            Some("arn:aws:codewhisperer:us-east-1:123:profile/ABC".to_string());
        let config = Config::default();
        let ctx = RequestContext {
            credentials: &credentials,
            token: "tok",
            machine_id: "machine123",
            config: &config,
        };
        let endpoint = IdeEndpoint::new();

        let url = endpoint.mcp_url(&ctx);
        assert_eq!(url, "https://runtime.us-east-1.kiro.dev");
        assert_eq!(endpoint.mcp_content_type(), "application/x-amz-json-1.0");

        let req = endpoint
            .decorate_mcp(reqwest::Client::new().post(&url), &ctx)
            .build()
            .unwrap();
        let header = |name: &str| {
            req.headers()
                .get(name)
                .map(|v| v.to_str().unwrap().to_string())
        };
        assert_eq!(
            header("x-amz-target").as_deref(),
            Some("KiroRuntimeService.InvokeMCP")
        );
        assert_eq!(header("TokenType").as_deref(), Some("SSO_OIDC"));
        assert_eq!(header("amz-sdk-request").as_deref(), Some("attempt=1; max=3"));
        assert_eq!(
            header("x-amz-user-agent"),
            Some(format!(
                "aws-sdk-js/1.0.0 KiroIDE-{}-machine123-KAS/{}",
                KIRO_VERSION, KAS_VERSION
            ))
        );
        assert!(
            header("user-agent")
                .unwrap()
                .contains("api/kiroruntime#1.0.0 m/N KiroIDE-")
        );
        assert!(header("x-amzn-kiro-profile-arn").is_none());
        assert!(header("x-kiro-attempt").is_none());

        let body = endpoint.transform_mcp_body(
            r#"{"id":"web_search_x","jsonrpc":"2.0","method":"tools/call","params":{"name":"web_search","arguments":{"query":"q"}}}"#,
            &ctx,
        );
        let json: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            json["profileArn"],
            "arn:aws:codewhisperer:us-east-1:123:profile/ABC"
        );
        assert_eq!(json["params"]["arguments"]["query"], "q");
    }

    #[test]
    fn test_inject_profile_arn_with_some() {
        let body = r#"{"conversationState":{"conversationId":"c1"}}"#;
        let arn = Some("arn:aws:codewhisperer:us-east-1:123:profile/ABC".to_string());
        let result = inject_profile_arn(body, &arn);
        let json: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            json["profileArn"],
            "arn:aws:codewhisperer:us-east-1:123:profile/ABC"
        );
        assert_eq!(json["conversationState"]["conversationId"], "c1");
    }

    #[test]
    fn test_inject_profile_arn_with_none() {
        let body = r#"{"conversationState":{"conversationId":"c1"}}"#;
        let result = inject_profile_arn(body, &None);
        let json: Value = serde_json::from_str(&result).unwrap();
        assert!(json.get("profileArn").is_none());
        assert_eq!(json["conversationState"]["conversationId"], "c1");
    }

    #[test]
    fn test_inject_profile_arn_overwrites_existing() {
        let body = r#"{"conversationState":{},"profileArn":"old-arn"}"#;
        let arn = Some("new-arn".to_string());
        let result = inject_profile_arn(body, &arn);
        let json: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(json["profileArn"], "new-arn");
    }

    #[test]
    fn test_inject_profile_arn_invalid_json() {
        let body = "not-valid-json";
        let arn = Some("arn:test".to_string());
        let result = inject_profile_arn(body, &arn);
        assert_eq!(result, "not-valid-json");
    }
}
