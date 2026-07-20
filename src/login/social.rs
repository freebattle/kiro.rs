//! Kiro IDE Social 登录流程（Portal PKCE OAuth）
//!
//! 复现 Kiro IDE 的 portal-auth-provider 流程：
//! 1. 生成 PKCE code_verifier + code_challenge
//! 2. 启本地 HTTP 回调服务器
//! 3. 返回 portal URL 供用户在浏览器完成登录
//! 4. 捕获回调中的 authorization code
//! 5. 用 code + code_verifier 换取 access_token + refresh_token

use std::net::TcpListener;

use base64::{Engine as _, engine::general_purpose};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot};

use crate::http_client::{ProxyConfig, build_client, build_client_no_redirect};
use crate::kiro::model::credentials::validate_external_idp_endpoint;
use crate::kiro::model::token_refresh::{
    ExternalIdpRefreshResponse, SocialCreateTokenRequest, SocialCreateTokenResponse,
};
use crate::model::config::{Config, KIRO_VERSION};

/// Portal 认证 URL（Kiro 网页版入口）
pub const KIRO_PORTAL_URL: &str = "https://app.kiro.dev";

/// Kiro auth service 默认端点
pub const KIRO_AUTH_ENDPOINT: &str = "https://prod.us-east-1.auth.desktop.kiro.dev";

/// Hosted Kiro SSO 企业 IdP 第二段固定回调地址（与 Kiro-Go / Kiro IDE 行为对齐）。
const EXTERNAL_IDP_REDIRECT_URI: &str = "http://localhost:3128/oauth/callback";

/// 与 IDE 一致的本地回调端口候选列表
const CALLBACK_PORTS: &[u16] = &[
    3128, 4649, 6588, 8008, 9091, 49153, 50153, 51153, 52153, 53153,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OAuthCallbackKind {
    AuthorizationCode,
    ExternalIdpDescriptor,
}

/// OAuth 回调数据
#[derive(Debug, Clone)]
pub struct OAuthCallbackData {
    pub kind: OAuthCallbackKind,
    pub code: String,
    pub login_option: String,
    pub path: String,
    /// OAuth state 参数（用于 CSRF 验证）
    pub state: String,
    pub issuer_url: Option<String>,
    pub client_id: Option<String>,
    pub scopes: Option<String>,
    pub login_hint: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ExternalIdpAuthorization {
    pub authorization_url: String,
    pub state: String,
    pub code_verifier: String,
    pub token_endpoint: String,
    pub issuer_url: String,
    pub client_id: String,
    pub scopes: String,
    pub redirect_uri: String,
}

#[derive(Debug, Clone)]
pub struct ExternalIdpToken {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: Option<i64>,
}

/// 回调服务器关闭句柄
///
/// Drop 时自动向服务器发送关闭信号，服务器退出监听循环并释放端口。
pub struct ServerHandle {
    _shutdown_tx: oneshot::Sender<()>,
}

/// 启动本地回调服务器，返回端口号和关闭句柄
///
/// 关闭句柄 drop 时服务器自动停止。当收到有效的 OAuth 回调时，通过 channel 发送回调数据。
pub fn start_callback_server(
    tx: mpsc::Sender<OAuthCallbackData>,
) -> anyhow::Result<(u16, ServerHandle)> {
    // 直接持有已绑定的 socket，避免 probe-and-bind 的 TOCTOU 竞态
    let (port, std_listener) = bind_available_port()?;

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    tokio::spawn(async move {
        run_callback_server(std_listener, tx, shutdown_rx).await;
    });

    Ok((
        port,
        ServerHandle {
            _shutdown_tx: shutdown_tx,
        },
    ))
}

fn bind_available_port() -> anyhow::Result<(u16, std::net::TcpListener)> {
    for &port in CALLBACK_PORTS {
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => {
                listener.set_nonblocking(true)?;
                return Ok((port, listener));
            }
            Err(_) => continue,
        }
    }
    anyhow::bail!(
        "所有回调端口均被占用，请确保没有其他程序占用 {:?}",
        CALLBACK_PORTS
    )
}

async fn run_callback_server(
    std_listener: std::net::TcpListener,
    tx: mpsc::Sender<OAuthCallbackData>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let port = std_listener.local_addr().map(|a| a.port()).unwrap_or(0);
    let listener = match TcpListener::from_std(std_listener) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("Social 回调服务器初始化失败 (port {}): {}", port, e);
            return;
        }
    };

    tracing::info!("Social 回调服务器已启动: http://127.0.0.1:{}", port);

    // 只等待一次成功的回调，或关闭信号
    loop {
        let (mut stream, _addr) = tokio::select! {
            result = listener.accept() => match result {
                Ok(s) => s,
                Err(_) => break,
            },
            _ = &mut shutdown_rx => {
                tracing::info!("Social 回调服务器收到关闭信号，端口 {} 已释放", port);
                break;
            }
        };

        let mut buf = vec![0u8; 4096];
        let n = match stream.read(&mut buf).await {
            Ok(n) => n,
            Err(_) => continue,
        };

        let request = String::from_utf8_lossy(&buf[..n]);
        let first_line = request.lines().next().unwrap_or("");

        // GET /oauth/callback?... HTTP/1.1
        if let Some(path_and_query) = first_line.strip_prefix("GET ").and_then(|s| {
            s.strip_suffix(" HTTP/1.1")
                .or_else(|| s.strip_suffix(" HTTP/1.0"))
        }) {
            if let Some(callback) = parse_callback(path_and_query) {
                let is_descriptor = callback.kind == OAuthCallbackKind::ExternalIdpDescriptor;
                let body = if is_descriptor {
                    "<html><head><meta charset='utf-8'><title>继续登录</title></head><body style='font-family:sans-serif;text-align:center;padding:60px'><h2>&#8635; 继续登录</h2><p>已收到企业 SSO 中间链接，请返回 Kiro Admin UI 打开下一段登录链接。</p><p style='color:#888;font-size:13px'>此标签页可以关闭。</p></body></html>"
                } else {
                    "<html><head><meta charset='utf-8'><title>登录成功</title></head><body style='font-family:sans-serif;text-align:center;padding:60px'><h2>&#10003; 登录成功</h2><p>Token 已更新，请返回 Kiro Admin UI。</p><p style='color:#888;font-size:13px'>此标签页可以关闭。</p></body></html>"
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.flush().await;

                let _ = tx.send(callback).await;
                if !is_descriptor {
                    break;
                }
                continue;
            } else if path_and_query.starts_with("/oauth/callback")
                || path_and_query.starts_with("/signin/callback")
            {
                // 有 error 参数的回调
                let error_msg = path_and_query
                    .split('?')
                    .nth(1)
                    .and_then(|q| {
                        let p = parse_query_string(q);
                        p.get("error_description")
                            .or_else(|| p.get("error"))
                            .cloned()
                    })
                    .unwrap_or_else(|| "未知错误".to_string());

                let body = format!(
                    "<html><head><meta charset='utf-8'><title>登录失败</title></head><body style='font-family:sans-serif;text-align:center;padding:60px'><h2>&#10007; 登录失败</h2><p>{}</p><p style='color:#888;font-size:13px'>请关闭此标签页并重试。</p></body></html>",
                    error_msg
                );
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.flush().await;
                break;
            }
        }

        // 其他请求返回 404
        let _ = stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
            .await;
    }
}

fn parse_callback(path_and_query: &str) -> Option<OAuthCallbackData> {
    let (path, query) = if let Some(idx) = path_and_query.find('?') {
        (&path_and_query[..idx], &path_and_query[idx + 1..])
    } else {
        return None;
    };

    if path != "/oauth/callback" && path != "/signin/callback" {
        return None;
    }

    let params = parse_query_string(query);

    // 必须没有 error；企业 SSO 第一段中间链接可以没有 code。
    if params.contains_key("error") {
        return None;
    }

    let login_option = params.get("login_option").cloned().unwrap_or_default();
    let issuer_url = params.get("issuer_url").filter(|v| !v.is_empty()).cloned();
    let client_id = params.get("client_id").filter(|v| !v.is_empty()).cloned();
    if path != "/oauth/callback"
        && (login_option.eq_ignore_ascii_case("external_idp") || issuer_url.is_some())
        && !params.contains_key("code")
    {
        return Some(OAuthCallbackData {
            kind: OAuthCallbackKind::ExternalIdpDescriptor,
            code: String::new(),
            login_option,
            path: path.to_string(),
            state: params.get("state").cloned().unwrap_or_default(),
            issuer_url,
            client_id,
            scopes: params.get("scopes").filter(|v| !v.is_empty()).cloned(),
            login_hint: params.get("login_hint").filter(|v| !v.is_empty()).cloned(),
        });
    }

    let code = params.get("code")?.clone();
    let login_option = params.get("login_option").cloned().unwrap_or_default();
    let state = params.get("state").cloned().unwrap_or_default();

    Some(OAuthCallbackData {
        kind: OAuthCallbackKind::AuthorizationCode,
        code,
        login_option,
        path: path.to_string(),
        state,
        issuer_url: None,
        client_id: None,
        scopes: None,
        login_hint: None,
    })
}

/// base64url 编码（无填充），与 Kiro IDE 行为一致
fn base64url_encode(data: &[u8]) -> String {
    // 标准 base64 → 替换 +/= 为 base64url 规范
    let b64 = base64_encode_standard(data);
    b64.replace('+', "-").replace('/', "_").replace('=', "")
}

/// 标准 base64 编码（用于内部转换）
fn base64_encode_standard(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = if chunk.len() > 1 {
            chunk[1] as usize
        } else {
            0
        };
        let b2 = if chunk.len() > 2 {
            chunk[2] as usize
        } else {
            0
        };
        out.push(CHARS[b0 >> 2] as char);
        out.push(CHARS[((b0 & 3) << 4) | (b1 >> 4)] as char);
        if chunk.len() > 1 {
            out.push(CHARS[((b1 & 0xf) << 2) | (b2 >> 6)] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(CHARS[b2 & 0x3f] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// 生成 PKCE code_verifier 和 code_challenge
pub fn generate_pkce() -> (String, String) {
    // 32 字节随机数作为 verifier（与 IDE crypto.randomBytes(32).toString("base64url") 等价）
    let mut bytes = [0u8; 32];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = fastrand::u8(..).wrapping_add(i as u8);
    }
    // 使用 uuid v4 的随机性来增强
    let uuid_bytes = uuid::Uuid::new_v4().as_bytes().to_owned();
    for (i, b) in bytes.iter_mut().enumerate() {
        *b ^= uuid_bytes[i % 16];
    }

    let verifier = base64url_encode(&bytes);

    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let digest = hasher.finalize();
    let challenge = base64url_encode(&digest);

    (verifier, challenge)
}

/// 构建供用户在浏览器中访问的 portal URL
pub fn build_portal_url(state: &str, code_challenge: &str, redirect_uri: &str) -> String {
    let params = format!(
        "state={}&code_challenge={}&code_challenge_method=S256&redirect_uri={}&redirect_from=KiroIDE",
        urlencoding::encode(state),
        urlencoding::encode(code_challenge),
        urlencoding::encode(redirect_uri),
    );
    format!("{}/signin?{}", KIRO_PORTAL_URL, params)
}

/// 从 Kiro portal 返回的企业 SSO 描述符构造第二段 IdP 授权链接。
pub async fn build_external_idp_authorization(
    descriptor: &OAuthCallbackData,
    _redirect_base_uri: &str,
    config: &Config,
    proxy: Option<&ProxyConfig>,
) -> anyhow::Result<ExternalIdpAuthorization> {
    if descriptor.kind != OAuthCallbackKind::ExternalIdpDescriptor {
        anyhow::bail!("不是企业 SSO 中间链接");
    }

    let issuer_url = descriptor
        .issuer_url
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("企业 SSO 中间链接缺少 issuer_url"))?;
    let client_id = descriptor
        .client_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("企业 SSO 中间链接缺少 client_id"))?;
    let scopes = descriptor.scopes.as_deref().unwrap_or("").trim();

    let (auth_endpoint, token_endpoint) = oidc_discover(issuer_url, config, proxy).await?;
    let (code_verifier, code_challenge) = generate_pkce();
    let state = uuid::Uuid::new_v4().to_string();
    let redirect_uri = EXTERNAL_IDP_REDIRECT_URI.to_string();
    let authorization_url = external_idp_authorize_url(
        &auth_endpoint,
        client_id,
        &redirect_uri,
        scopes,
        &code_challenge,
        &state,
        descriptor.login_hint.as_deref().unwrap_or(""),
    );

    Ok(ExternalIdpAuthorization {
        authorization_url,
        state,
        code_verifier,
        token_endpoint,
        issuer_url: issuer_url.to_string(),
        client_id: client_id.to_string(),
        scopes: scopes.to_string(),
        redirect_uri,
    })
}

#[derive(Debug, Deserialize)]
struct OidcDiscoveryDocument {
    authorization_endpoint: String,
    token_endpoint: String,
}

async fn oidc_discover(
    issuer_url: &str,
    config: &Config,
    proxy: Option<&ProxyConfig>,
) -> anyhow::Result<(String, String)> {
    validate_external_idp_endpoint(issuer_url)
        .map_err(|e| anyhow::anyhow!("企业 SSO issuer 被拒绝: {}", e))?;
    let doc_url = format!(
        "{}/.well-known/openid-configuration",
        issuer_url.trim_end_matches('/')
    );
    let client = build_client_no_redirect(proxy, 30)?;
    let response = client
        .get(&doc_url)
        .header("Accept", "application/json")
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("OIDC discovery 失败: HTTP {}", status.as_u16());
    }
    let doc: OidcDiscoveryDocument = response.json().await?;
    if doc.authorization_endpoint.trim().is_empty() || doc.token_endpoint.trim().is_empty() {
        anyhow::bail!("OIDC discovery 缺少 authorization_endpoint 或 token_endpoint");
    }
    validate_external_idp_endpoint(&doc.authorization_endpoint)
        .map_err(|e| anyhow::anyhow!("discovered authorization_endpoint 被拒绝: {}", e))?;
    validate_external_idp_endpoint(&doc.token_endpoint)
        .map_err(|e| anyhow::anyhow!("discovered token_endpoint 被拒绝: {}", e))?;
    Ok((doc.authorization_endpoint, doc.token_endpoint))
}

fn external_idp_authorize_url(
    auth_endpoint: &str,
    client_id: &str,
    redirect_uri: &str,
    scopes: &str,
    challenge: &str,
    state: &str,
    login_hint: &str,
) -> String {
    let mut query = vec![
        ("client_id", client_id),
        ("response_type", "code"),
        ("redirect_uri", redirect_uri),
        ("scope", scopes),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("response_mode", "query"),
        ("state", state),
    ]
    .into_iter()
    .map(|(key, value)| format!("{}={}", key, urlencoding::encode(value)))
    .collect::<Vec<_>>();

    if !login_hint.trim().is_empty() {
        query.push(format!(
            "login_hint={}",
            urlencoding::encode(login_hint.trim())
        ));
    }

    format!("{}?{}", auth_endpoint, query.join("&"))
}

/// 简易 query string 解析（不依赖 url crate）
fn parse_query_string(query: &str) -> std::collections::HashMap<String, String> {
    query
        .split('&')
        .filter_map(|pair| {
            let mut iter = pair.splitn(2, '=');
            let key = iter.next()?.to_string();
            let val = iter
                .next()
                .map(|v| {
                    // 简单的 percent-decode（处理 %XX 和 + 号）
                    let with_space = v.replace('+', " ");
                    urlencoding::decode(&with_space)
                        .map(|s| s.into_owned())
                        .unwrap_or_else(|_| with_space)
                })
                .unwrap_or_default();
            Some((key, val))
        })
        .collect()
}

/// 用 authorization code 换取 access_token + refresh_token
pub async fn exchange_code_for_token(
    auth_endpoint: &str,
    code: &str,
    code_verifier: &str,
    full_redirect_uri: &str,
    config: &Config,
    proxy: Option<&ProxyConfig>,
) -> anyhow::Result<SocialCreateTokenResponse> {
    let url = format!("{}/oauth/token", auth_endpoint);
    let client = build_client(proxy, 30)?;

    let body = SocialCreateTokenRequest {
        code: code.to_string(),
        code_verifier: code_verifier.to_string(),
        redirect_uri: full_redirect_uri.to_string(),
        invitation_code: None,
    };

    let _ = config; // 预留与全局配置扩展对齐
    let user_agent = format!("KiroIDE-{}", KIRO_VERSION);

    let resp = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("User-Agent", &user_agent)
        .header("host", auth_endpoint.trim_start_matches("https://"))
        .json(&body)
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Social token 交换失败 {}: {}", status, body_text);
    }

    resp.json::<SocialCreateTokenResponse>()
        .await
        .map_err(|e| anyhow::anyhow!("解析 Social token 响应失败: {}", e))
}

/// 用 External IdP authorization code 换取 access_token + refresh_token。
pub async fn exchange_external_idp_code(
    token_endpoint: &str,
    client_id: &str,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
    scopes: &str,
    config: &Config,
    proxy: Option<&ProxyConfig>,
) -> anyhow::Result<ExternalIdpToken> {
    validate_external_idp_endpoint(token_endpoint)
        .map_err(|e| anyhow::anyhow!("External IdP tokenEndpoint 被拒绝: {}", e))?;

    let mut form = vec![
        ("client_id", client_id),
        ("grant_type", "authorization_code"),
        ("code", code.trim()),
        ("redirect_uri", redirect_uri),
        ("code_verifier", code_verifier),
    ];
    if !scopes.trim().is_empty() {
        form.push(("scope", scopes.trim()));
    }

    let _ = config;
    let client = build_client(proxy, 30)?;
    let response = client
        .post(token_endpoint)
        .header("Accept", "application/json")
        .form(&form)
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let data: ExternalIdpRefreshResponse = serde_json::from_str(&body).unwrap_or_default();

    let access_token = data.access_token.filter(|v| !v.trim().is_empty());
    let refresh_token = data.refresh_token.filter(|v| !v.trim().is_empty());
    match (status.is_success(), access_token, refresh_token) {
        (true, Some(access_token), Some(refresh_token)) => Ok(ExternalIdpToken {
            access_token,
            refresh_token,
            expires_in: data.expires_in,
        }),
        _ => {
            if let Some(err) = data.error {
                anyhow::bail!(
                    "External IdP token 交换失败 {}: {}: {}",
                    status,
                    err,
                    data.error_description.unwrap_or_default()
                );
            }
            anyhow::bail!("External IdP token 交换失败 {}: {}", status, body);
        }
    }
}

/// 最佳努力从 JWT access token 提取账号邮箱。
pub fn extract_email_from_jwt(access_token: &str) -> Option<String> {
    let payload = access_token.split('.').nth(1)?;
    let decoded = general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| general_purpose::URL_SAFE.decode(payload))
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    for claim in ["email", "preferred_username", "upn"] {
        if let Some(value) = claims
            .get(claim)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            return Some(value.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_callback_accepts_external_idp_descriptor_without_code() {
        let callback = parse_callback(
            "/signin/callback?login_option=external_idp&issuer_url=https%3A%2F%2Flogin.microsoftonline.com%2Ftenant%2Fv2.0&client_id=client-123&scopes=api%3A%2F%2Fclient-123%2Fcodewhisperer%3Aconversations+offline_access&login_hint=user%40example.com&state=portal-state",
        )
        .expect("descriptor should parse");

        assert_eq!(callback.kind, OAuthCallbackKind::ExternalIdpDescriptor);
        assert_eq!(callback.code, "");
        assert_eq!(callback.path, "/signin/callback");
        assert_eq!(callback.state, "portal-state");
        assert_eq!(
            callback.issuer_url.as_deref(),
            Some("https://login.microsoftonline.com/tenant/v2.0")
        );
        assert_eq!(callback.client_id.as_deref(), Some("client-123"));
        assert_eq!(
            callback.scopes.as_deref(),
            Some("api://client-123/codewhisperer:conversations offline_access")
        );
        assert_eq!(callback.login_hint.as_deref(), Some("user@example.com"));
    }

    #[test]
    fn parse_callback_keeps_oauth_callback_as_final_code() {
        let callback = parse_callback(
            "/oauth/callback?code=final-code&state=idp-state&login_option=external_idp",
        )
        .expect("final callback should parse");

        assert_eq!(callback.kind, OAuthCallbackKind::AuthorizationCode);
        assert_eq!(callback.code, "final-code");
        assert_eq!(callback.path, "/oauth/callback");
        assert_eq!(callback.state, "idp-state");
        assert_eq!(callback.login_option, "external_idp");
        assert!(callback.issuer_url.is_none());
    }

    #[test]
    fn external_idp_authorize_url_matches_fixed_kiro_redirect() {
        let url = external_idp_authorize_url(
            "https://login.microsoftonline.com/tenant/oauth2/v2.0/authorize",
            "client-123",
            EXTERNAL_IDP_REDIRECT_URI,
            "api://client-123/codewhisperer:conversations offline_access",
            "challenge",
            "state-123",
            "user@example.com",
        );

        let query = url
            .split_once('?')
            .map(|(_, query)| query)
            .expect("authorize URL query");
        let params = parse_query_string(query);
        assert_eq!(
            params.get("client_id").map(String::as_str),
            Some("client-123")
        );
        assert_eq!(
            params.get("redirect_uri").map(String::as_str),
            Some(EXTERNAL_IDP_REDIRECT_URI)
        );
        assert_eq!(
            params.get("response_type").map(String::as_str),
            Some("code")
        );
        assert_eq!(
            params.get("code_challenge").map(String::as_str),
            Some("challenge")
        );
        assert_eq!(params.get("state").map(String::as_str), Some("state-123"));
        assert_eq!(
            params.get("login_hint").map(String::as_str),
            Some("user@example.com")
        );
    }

    #[test]
    fn extract_email_from_jwt_uses_azure_username_claims() {
        let payload = general_purpose::URL_SAFE_NO_PAD
            .encode(br#"{"preferred_username":"user@example.com"}"#);
        let token = format!("header.{}.sig", payload);

        assert_eq!(
            extract_email_from_jwt(&token).as_deref(),
            Some("user@example.com")
        );
    }
}
