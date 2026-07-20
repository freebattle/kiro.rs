//! 登录授权业务编排
//!
//! 与主请求链路隔离：只通过 MultiTokenManager 写凭据，不介入 chat/messages 路径。

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use parking_lot::Mutex;
use uuid::Uuid;

use crate::admin::AdminServiceError;
use crate::http_client::ProxyConfig;
use crate::kiro::model::credentials::KiroCredentials;
use crate::kiro::token_manager::MultiTokenManager;
use crate::login::idc::{self, BUILDER_ID_START_URL};
use crate::login::social;
use crate::login::types::{
    CompleteSocialLoginRequest, PollLoginResponse, StartIdcLoginRequest, StartIdcLoginResponse,
    StartSocialLoginRequest, StartSocialLoginResponse,
};

struct SocialAuthSession {
    auth_endpoint: String,
    state: String,
    code_verifier: String,
    redirect_uri: String,
    expires_at: DateTime<Utc>,
    callback_rx: tokio::sync::Mutex<tokio::sync::mpsc::Receiver<social::OAuthCallbackData>>,
    external_idp: Option<ExternalIdpLoginState>,
    cred_template: KiroCredentials,
    proxy: Option<ProxyConfig>,
    _server_handle: social::ServerHandle,
    relogin_target_id: Option<u64>,
}

#[derive(Debug, Clone)]
struct ExternalIdpLoginState {
    state: String,
    code_verifier: String,
    token_endpoint: String,
    issuer_url: String,
    client_id: String,
    scopes: String,
    redirect_uri: String,
    authorization_url: String,
}

struct IdcAuthSession {
    region: String,
    client_id: String,
    client_secret: String,
    device_code: String,
    expires_at: DateTime<Utc>,
    poll_interval: i64,
    cred_template: KiroCredentials,
    proxy: Option<ProxyConfig>,
    relogin_target_id: Option<u64>,
}

/// 登录授权服务（内存会话，不落盘）
pub struct LoginService {
    token_manager: Arc<MultiTokenManager>,
    global_proxy: Option<ProxyConfig>,
    idc_sessions: Mutex<HashMap<String, IdcAuthSession>>,
    social_sessions: Mutex<HashMap<String, SocialAuthSession>>,
}

impl LoginService {
    pub fn new(token_manager: Arc<MultiTokenManager>, global_proxy: Option<ProxyConfig>) -> Self {
        Self {
            token_manager,
            global_proxy,
            idc_sessions: Mutex::new(HashMap::new()),
            social_sessions: Mutex::new(HashMap::new()),
        }
    }

    fn resolve_proxy(&self, proxy_url: Option<&str>) -> Option<ProxyConfig> {
        proxy_url
            .map(ProxyConfig::new)
            .or_else(|| self.global_proxy.clone())
    }

    // ── Social 登录 ──────────────────────────────────────────────────────────

    pub async fn start_social_login(
        &self,
        req: StartSocialLoginRequest,
    ) -> Result<StartSocialLoginResponse, AdminServiceError> {
        self.start_social_login_inner(req, None).await
    }

    pub async fn start_social_relogin(
        &self,
        target_id: u64,
        req: StartSocialLoginRequest,
    ) -> Result<StartSocialLoginResponse, AdminServiceError> {
        self.ensure_credential_exists(target_id)?;
        self.start_social_login_inner(req, Some(target_id)).await
    }

    async fn start_social_login_inner(
        &self,
        req: StartSocialLoginRequest,
        relogin_target_id: Option<u64>,
    ) -> Result<StartSocialLoginResponse, AdminServiceError> {
        let proxy = self.resolve_proxy(req.proxy_url.as_deref());
        let auth_endpoint = req
            .auth_endpoint
            .unwrap_or_else(|| social::KIRO_AUTH_ENDPOINT.to_string());

        let (code_verifier, code_challenge) = social::generate_pkce();
        let state = Uuid::new_v4().to_string();
        let (tx, rx) = tokio::sync::mpsc::channel::<social::OAuthCallbackData>(4);

        let (port, server_handle) = social::start_callback_server(tx)
            .map_err(|e| AdminServiceError::InternalError(e.to_string()))?;

        let redirect_uri = format!("http://127.0.0.1:{}", port);
        let portal_url = social::build_portal_url(&state, &code_challenge, &redirect_uri);
        let expires_at = Utc::now() + Duration::minutes(10);
        let session_id = Uuid::new_v4().to_string();

        let cred_template = if relogin_target_id.is_some() {
            KiroCredentials::default()
        } else {
            KiroCredentials {
                auth_method: Some("social".to_string()),
                priority: req.priority,
                email: req.email,
                proxy_url: req.proxy_url,
                ..Default::default()
            }
        };

        let session = SocialAuthSession {
            auth_endpoint,
            state,
            code_verifier,
            redirect_uri,
            expires_at,
            callback_rx: tokio::sync::Mutex::new(rx),
            external_idp: None,
            cred_template,
            proxy,
            _server_handle: server_handle,
            relogin_target_id,
        };

        self.social_sessions
            .lock()
            .insert(session_id.clone(), session);

        Ok(StartSocialLoginResponse {
            session_id,
            portal_url,
            expires_at: expires_at.to_rfc3339(),
        })
    }

    pub async fn poll_social_login(
        &self,
        session_id: &str,
    ) -> Result<PollLoginResponse, AdminServiceError> {
        use tokio::sync::mpsc::error::TryRecvError;

        enum PollOutcome {
            Expired,
            Closed,
            Pending,
            Received(social::OAuthCallbackData),
        }

        let outcome = {
            let sessions = self.social_sessions.lock();
            let Some(session) = sessions.get(session_id) else {
                return Err(AdminServiceError::NotFound { id: 0 });
            };

            if Utc::now() >= session.expires_at {
                PollOutcome::Expired
            } else {
                match session.callback_rx.try_lock() {
                    Ok(mut rx) => match rx.try_recv() {
                        Ok(data) => PollOutcome::Received(data),
                        Err(TryRecvError::Empty) => PollOutcome::Pending,
                        Err(TryRecvError::Disconnected) => PollOutcome::Closed,
                    },
                    Err(_) => PollOutcome::Pending,
                }
            }
        };

        match outcome {
            PollOutcome::Pending => Ok(PollLoginResponse::Pending),
            PollOutcome::Expired => {
                self.social_sessions.lock().remove(session_id);
                Ok(PollLoginResponse::Expired)
            }
            PollOutcome::Closed => {
                self.social_sessions.lock().remove(session_id);
                Err(AdminServiceError::InternalError(
                    "Social 登录回调服务器已关闭，请重新发起登录".to_string(),
                ))
            }
            PollOutcome::Received(callback) => {
                self.do_complete_social_login(session_id, callback).await
            }
        }
    }

    pub async fn complete_social_login(
        &self,
        session_id: &str,
        req: CompleteSocialLoginRequest,
    ) -> Result<PollLoginResponse, AdminServiceError> {
        {
            let sessions = self.social_sessions.lock();
            let s = sessions
                .get(session_id)
                .ok_or(AdminServiceError::NotFound { id: 0 })?;
            if Utc::now() >= s.expires_at {
                return Ok(PollLoginResponse::Expired);
            }
        }

        let callback = social::OAuthCallbackData {
            kind: if req.code.as_deref().unwrap_or("").trim().is_empty()
                && (req.issuer_url.is_some()
                    || req.login_option.eq_ignore_ascii_case("external_idp"))
            {
                social::OAuthCallbackKind::ExternalIdpDescriptor
            } else {
                social::OAuthCallbackKind::AuthorizationCode
            },
            code: req.code.unwrap_or_default(),
            login_option: req.login_option,
            path: req.path,
            state: req.state.unwrap_or_default(),
            issuer_url: req.issuer_url,
            client_id: req.client_id,
            scopes: req.scopes,
            login_hint: req.login_hint,
        };
        self.do_complete_social_login(session_id, callback).await
    }

    async fn do_complete_social_login(
        &self,
        session_id: &str,
        callback: social::OAuthCallbackData,
    ) -> Result<PollLoginResponse, AdminServiceError> {
        if callback.kind == social::OAuthCallbackKind::ExternalIdpDescriptor {
            return self
                .handle_external_idp_descriptor(session_id, callback)
                .await;
        }

        let is_external_idp_final = {
            let sessions = self.social_sessions.lock();
            let s = sessions
                .get(session_id)
                .ok_or(AdminServiceError::NotFound { id: 0 })?;
            if Utc::now() >= s.expires_at {
                return Ok(PollLoginResponse::Expired);
            }
            s.external_idp.is_some() && callback.path == "/oauth/callback"
        };

        if is_external_idp_final {
            self.finish_external_idp_login(session_id, callback).await
        } else {
            self.finish_social_code_login(session_id, callback).await
        }
    }

    async fn handle_external_idp_descriptor(
        &self,
        session_id: &str,
        callback: social::OAuthCallbackData,
    ) -> Result<PollLoginResponse, AdminServiceError> {
        let (redirect_uri, proxy) = {
            let sessions = self.social_sessions.lock();
            let s = sessions
                .get(session_id)
                .ok_or(AdminServiceError::NotFound { id: 0 })?;
            if Utc::now() >= s.expires_at {
                return Ok(PollLoginResponse::Expired);
            }
            if !callback.state.is_empty() && callback.state != s.state {
                return Err(AdminServiceError::InternalError(
                    "OAuth state 不匹配，请重新发起登录".to_string(),
                ));
            }
            if let Some(existing) = &s.external_idp {
                return Ok(PollLoginResponse::Continue {
                    next_url: existing.authorization_url.clone(),
                });
            }
            (s.redirect_uri.clone(), s.proxy.clone())
        };

        let config = self.token_manager.config();
        let authorization = social::build_external_idp_authorization(
            &callback,
            &redirect_uri,
            config,
            proxy.as_ref(),
        )
        .await
        .map_err(|e| AdminServiceError::InternalError(e.to_string()))?;

        let next_url = authorization.authorization_url.clone();
        let mut sessions = self.social_sessions.lock();
        let s = sessions
            .get_mut(session_id)
            .ok_or(AdminServiceError::NotFound { id: 0 })?;
        if Utc::now() >= s.expires_at {
            return Ok(PollLoginResponse::Expired);
        }
        if s.external_idp.is_none() {
            s.external_idp = Some(ExternalIdpLoginState {
                state: authorization.state,
                code_verifier: authorization.code_verifier,
                token_endpoint: authorization.token_endpoint,
                issuer_url: authorization.issuer_url,
                client_id: authorization.client_id,
                scopes: authorization.scopes,
                redirect_uri: authorization.redirect_uri,
                authorization_url: next_url.clone(),
            });
        }

        Ok(PollLoginResponse::Continue { next_url })
    }

    async fn finish_social_code_login(
        &self,
        session_id: &str,
        callback: social::OAuthCallbackData,
    ) -> Result<PollLoginResponse, AdminServiceError> {
        if callback.code.trim().is_empty() {
            return Err(AdminServiceError::InternalError(
                "OAuth 回调缺少 code".to_string(),
            ));
        }

        {
            let sessions = self.social_sessions.lock();
            let s = sessions
                .get(session_id)
                .ok_or(AdminServiceError::NotFound { id: 0 })?;
            if Utc::now() >= s.expires_at {
                return Ok(PollLoginResponse::Expired);
            }
            if callback.state != s.state {
                return Err(AdminServiceError::InternalError(
                    "OAuth state 不匹配，请重新发起登录".to_string(),
                ));
            }
        }

        let session = self
            .social_sessions
            .lock()
            .remove(session_id)
            .ok_or(AdminServiceError::NotFound { id: 0 })?;

        let config = self.token_manager.config();
        let full_redirect_uri = if callback.login_option.is_empty() {
            format!("{}{}", session.redirect_uri, callback.path)
        } else {
            format!(
                "{}{}?login_option={}",
                session.redirect_uri,
                callback.path,
                urlencoding::encode(&callback.login_option),
            )
        };

        let token = social::exchange_code_for_token(
            &session.auth_endpoint,
            &callback.code,
            &session.code_verifier,
            &full_redirect_uri,
            config,
            session.proxy.as_ref(),
        )
        .await
        .map_err(|e| AdminServiceError::InternalError(e.to_string()))?;

        if let Some(target_id) = session.relogin_target_id {
            let refresh_token = token.refresh_token.ok_or_else(|| {
                AdminServiceError::InternalError(
                    "Social 登录未返回 refreshToken，无法更新凭据".to_string(),
                )
            })?;
            self.do_relogin_update(target_id, refresh_token)
                .map_err(|e| AdminServiceError::InternalError(e.to_string()))?;
            tracing::info!("Social 重新登录成功，凭据 #{} Token 已更新", target_id);
            return Ok(PollLoginResponse::Success {
                credential_id: target_id,
            });
        }

        let mut new_cred = session.cred_template;
        new_cred.access_token = Some(token.access_token);
        new_cred.refresh_token = token.refresh_token;
        new_cred.expires_at = token.expires_at.or_else(|| {
            token
                .expires_in
                .map(|secs| (Utc::now() + Duration::seconds(secs)).to_rfc3339())
        });
        if let Some(arn) = token.profile_arn {
            new_cred.profile_arn = Some(arn);
        }

        let credential_id = self
            .token_manager
            .add_credential(new_cred)
            .await
            .map_err(|e| AdminServiceError::InternalError(e.to_string()))?;

        tracing::info!("Social 登录成功，已添加凭据 #{}", credential_id);
        Ok(PollLoginResponse::Success { credential_id })
    }

    async fn finish_external_idp_login(
        &self,
        session_id: &str,
        callback: social::OAuthCallbackData,
    ) -> Result<PollLoginResponse, AdminServiceError> {
        if callback.code.trim().is_empty() {
            return Err(AdminServiceError::InternalError(
                "External IdP 回调缺少 code".to_string(),
            ));
        }

        let (session, external_idp) = {
            let mut sessions = self.social_sessions.lock();
            let s = sessions
                .get(session_id)
                .ok_or(AdminServiceError::NotFound { id: 0 })?;
            if Utc::now() >= s.expires_at {
                return Ok(PollLoginResponse::Expired);
            }
            let external_idp = s.external_idp.clone().ok_or_else(|| {
                AdminServiceError::InternalError(
                    "尚未收到企业 SSO 中间链接，请先粘贴第一段回调 URL".to_string(),
                )
            })?;
            if callback.state != external_idp.state {
                return Err(AdminServiceError::InternalError(
                    "External IdP state 不匹配，请重新发起登录".to_string(),
                ));
            }
            let session = sessions
                .remove(session_id)
                .ok_or(AdminServiceError::NotFound { id: 0 })?;
            (session, external_idp)
        };

        let config = self.token_manager.config();
        let token = social::exchange_external_idp_code(
            &external_idp.token_endpoint,
            &external_idp.client_id,
            &callback.code,
            &external_idp.code_verifier,
            &external_idp.redirect_uri,
            &external_idp.scopes,
            config,
            session.proxy.as_ref(),
        )
        .await
        .map_err(|e| AdminServiceError::InternalError(e.to_string()))?;

        if let Some(target_id) = session.relogin_target_id {
            self.do_external_idp_relogin_update(target_id, token, external_idp)
                .map_err(|e| AdminServiceError::InternalError(e.to_string()))?;
            tracing::info!(
                "External IdP 重新登录成功，凭据 #{} Token 已更新",
                target_id
            );
            return Ok(PollLoginResponse::Success {
                credential_id: target_id,
            });
        }

        let mut new_cred = session.cred_template;
        new_cred.auth_method = Some("external_idp".to_string());
        new_cred.provider = Some("AzureAD".to_string());
        new_cred.client_id = Some(external_idp.client_id);
        new_cred.client_secret = None;
        new_cred.token_endpoint = Some(external_idp.token_endpoint);
        new_cred.issuer_url = Some(external_idp.issuer_url);
        new_cred.scopes = Some(external_idp.scopes);
        new_cred.access_token = Some(token.access_token.clone());
        new_cred.refresh_token = Some(token.refresh_token);
        new_cred.expires_at = token
            .expires_in
            .map(|secs| (Utc::now() + Duration::seconds(secs)).to_rfc3339());
        if new_cred
            .email
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
        {
            new_cred.email = social::extract_email_from_jwt(&token.access_token);
        }

        let credential_id = self
            .token_manager
            .add_credential(new_cred)
            .await
            .map_err(|e| AdminServiceError::InternalError(e.to_string()))?;

        tracing::info!("External IdP 登录成功，已添加凭据 #{}", credential_id);
        Ok(PollLoginResponse::Success { credential_id })
    }

    // ── IdC 设备授权登录 ────────────────────────────────────────────────────

    pub async fn start_idc_login(
        &self,
        req: StartIdcLoginRequest,
    ) -> Result<StartIdcLoginResponse, AdminServiceError> {
        self.start_idc_login_inner(req, None).await
    }

    pub async fn start_idc_relogin(
        &self,
        target_id: u64,
        req: StartIdcLoginRequest,
    ) -> Result<StartIdcLoginResponse, AdminServiceError> {
        self.ensure_credential_exists(target_id)?;
        self.start_idc_login_inner(req, Some(target_id)).await
    }

    async fn start_idc_login_inner(
        &self,
        req: StartIdcLoginRequest,
        relogin_target_id: Option<u64>,
    ) -> Result<StartIdcLoginResponse, AdminServiceError> {
        let config = self.token_manager.config();
        let proxy = self.resolve_proxy(req.proxy_url.as_deref());
        let start_url = req.start_url.as_deref().unwrap_or(BUILDER_ID_START_URL);

        let reg = idc::register_client(&req.region, start_url, config, proxy.as_ref())
            .await
            .map_err(|e| AdminServiceError::InternalError(e.to_string()))?;

        let device = idc::start_device_authorization(
            &req.region,
            start_url,
            &reg.client_id,
            &reg.client_secret,
            config,
            proxy.as_ref(),
        )
        .await
        .map_err(|e| AdminServiceError::InternalError(e.to_string()))?;

        let expires_at = Utc::now() + Duration::seconds(device.expires_in);
        let session_id = Uuid::new_v4().to_string();

        let provider = if start_url == BUILDER_ID_START_URL {
            "BuilderId"
        } else {
            "Enterprise"
        };

        let cred_template = if relogin_target_id.is_some() {
            KiroCredentials::default()
        } else {
            // OIDC/SSO 区域只写入 authRegion，不写 region。
            // 登录成功后 add_credential 会立即 ListAvailableProfiles 拿 profileArn；
            // 后续 API 请求通过 profileArn 解析真实 api region。
            KiroCredentials {
                auth_method: Some("idc".to_string()),
                provider: Some(provider.to_string()),
                client_id: Some(reg.client_id.clone()),
                client_secret: Some(reg.client_secret.clone()),
                start_url: Some(start_url.to_string()),
                auth_region: Some(req.region.clone()),
                priority: req.priority,
                email: req.email,
                proxy_url: req.proxy_url,
                ..Default::default()
            }
        };

        let session = IdcAuthSession {
            region: req.region,
            client_id: reg.client_id,
            client_secret: reg.client_secret,
            device_code: device.device_code,
            expires_at,
            poll_interval: device.interval.max(5),
            cred_template,
            proxy,
            relogin_target_id,
        };

        let poll_interval = session.poll_interval;
        self.idc_sessions.lock().insert(session_id.clone(), session);

        Ok(StartIdcLoginResponse {
            session_id,
            user_code: device.user_code,
            verification_uri: device.verification_uri,
            verification_uri_complete: device.verification_uri_complete,
            expires_at: expires_at.to_rfc3339(),
            poll_interval,
        })
    }

    pub async fn poll_idc_login(
        &self,
        session_id: &str,
    ) -> Result<PollLoginResponse, AdminServiceError> {
        let (
            region,
            client_id,
            client_secret,
            device_code,
            proxy,
            cred_template,
            relogin_target_id,
        ) = {
            let sessions = self.idc_sessions.lock();
            let s = sessions
                .get(session_id)
                .ok_or(AdminServiceError::NotFound { id: 0 })?;

            if Utc::now() >= s.expires_at {
                return Ok(PollLoginResponse::Expired);
            }

            (
                s.region.clone(),
                s.client_id.clone(),
                s.client_secret.clone(),
                s.device_code.clone(),
                s.proxy.clone(),
                s.cred_template.clone(),
                s.relogin_target_id,
            )
        };

        let config = self.token_manager.config();
        match idc::poll_token(
            &region,
            &client_id,
            &client_secret,
            &device_code,
            config,
            proxy.as_ref(),
        )
        .await
        {
            idc::PollResult::Pending => Ok(PollLoginResponse::Pending),
            idc::PollResult::Expired => {
                self.idc_sessions.lock().remove(session_id);
                Ok(PollLoginResponse::Expired)
            }
            idc::PollResult::Error(e) => Err(AdminServiceError::InternalError(e.to_string())),
            idc::PollResult::Success(token) => {
                self.idc_sessions.lock().remove(session_id);

                if let Some(target_id) = relogin_target_id {
                    if let Some(refresh_token) = token.refresh_token {
                        self.do_relogin_update(target_id, refresh_token)
                            .map_err(|e| AdminServiceError::InternalError(e.to_string()))?;
                    }
                    tracing::info!("IdC 重新登录成功，凭据 #{} Token 已更新", target_id);
                    return Ok(PollLoginResponse::Success {
                        credential_id: target_id,
                    });
                }

                let mut new_cred = cred_template;
                new_cred.access_token = Some(token.access_token);
                new_cred.refresh_token = token.refresh_token;
                if let Some(secs) = token.expires_in {
                    new_cred.expires_at =
                        Some((Utc::now() + Duration::seconds(secs)).to_rfc3339());
                }

                let credential_id = self
                    .token_manager
                    .add_credential(new_cred)
                    .await
                    .map_err(|e| AdminServiceError::InternalError(e.to_string()))?;

                tracing::info!("IdC 设备授权登录成功，已添加凭据 #{}", credential_id);
                Ok(PollLoginResponse::Success { credential_id })
            }
        }
    }

    // ── 内部工具 ────────────────────────────────────────────────────────────

    fn ensure_credential_exists(&self, target_id: u64) -> Result<(), AdminServiceError> {
        let snapshot = self.token_manager.snapshot();
        if !snapshot.entries.iter().any(|e| e.id == target_id) {
            return Err(AdminServiceError::NotFound { id: target_id });
        }
        Ok(())
    }

    fn do_relogin_update(&self, target_id: u64, refresh_token: String) -> anyhow::Result<()> {
        self.token_manager.set_disabled(target_id, true)?;
        self.token_manager
            .update_refresh_token(target_id, refresh_token, None, None)?;
        self.token_manager.reset_and_enable(target_id)?;
        Ok(())
    }

    fn do_external_idp_relogin_update(
        &self,
        target_id: u64,
        token: social::ExternalIdpToken,
        external_idp: ExternalIdpLoginState,
    ) -> anyhow::Result<()> {
        self.token_manager.set_disabled(target_id, true)?;
        let expires_at = token
            .expires_in
            .map(|secs| (Utc::now() + Duration::seconds(secs)).to_rfc3339());
        self.token_manager.update_external_idp_relogin(
            target_id,
            token.refresh_token,
            Some(token.access_token),
            expires_at,
            external_idp.client_id,
            external_idp.token_endpoint,
            external_idp.issuer_url,
            external_idp.scopes,
        )?;
        self.token_manager.reset_and_enable(target_id)?;
        Ok(())
    }
}
