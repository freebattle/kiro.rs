//! 登录授权 API 类型

use serde::{Deserialize, Serialize};

// ============ IdC 设备授权登录 ============

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartIdcLoginRequest {
    pub region: String,
    #[serde(default)]
    pub start_url: Option<String>,
    #[serde(default)]
    pub priority: u32,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub proxy_url: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartIdcLoginResponse {
    pub session_id: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_uri_complete: Option<String>,
    pub expires_at: String,
    pub poll_interval: i64,
}

/// 轮询登录状态响应（Social / IdC 共用）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "status")]
pub enum PollLoginResponse {
    #[serde(rename = "pending")]
    Pending,
    #[serde(rename = "continue")]
    Continue { next_url: String },
    #[serde(rename = "success")]
    Success { credential_id: u64 },
    #[serde(rename = "expired")]
    Expired,
}

// ============ Social 登录（Portal PKCE OAuth） ============

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartSocialLoginRequest {
    #[serde(default)]
    pub priority: u32,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub proxy_url: Option<String>,
    #[serde(default)]
    pub auth_endpoint: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartSocialLoginResponse {
    pub session_id: String,
    pub portal_url: String,
    pub expires_at: String,
}

/// 手动完成 Social 登录（远程访问：粘贴回调 URL）
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompleteSocialLoginRequest {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub login_option: String,
    #[serde(default = "default_oauth_path")]
    pub path: String,
    #[serde(default)]
    pub issuer_url: Option<String>,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub scopes: Option<String>,
    #[serde(default)]
    pub login_hint: Option<String>,
}

fn default_oauth_path() -> String {
    "/oauth/callback".to_string()
}
