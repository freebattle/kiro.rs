//! 登录授权 HTTP handlers

use axum::{
    Json,
    extract::{Path, State},
    response::IntoResponse,
};

use crate::admin::middleware::AdminState;
use crate::login::types::{
    CompleteSocialLoginRequest, StartIdcLoginRequest, StartSocialLoginRequest,
};

type CredSessionPath = (u64, String);

/// POST /api/admin/auth/social/start
pub async fn start_social_login(
    State(state): State<AdminState>,
    Json(payload): Json<StartSocialLoginRequest>,
) -> impl IntoResponse {
    match state.login.start_social_login(payload).await {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => (e.status_code(), Json(e.into_response())).into_response(),
    }
}

/// POST /api/admin/auth/social/poll/:session_id
pub async fn poll_social_login(
    State(state): State<AdminState>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    match state.login.poll_social_login(&session_id).await {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => (e.status_code(), Json(e.into_response())).into_response(),
    }
}

/// POST /api/admin/auth/social/complete/:session_id
pub async fn complete_social_login(
    State(state): State<AdminState>,
    Path(session_id): Path<String>,
    Json(payload): Json<CompleteSocialLoginRequest>,
) -> impl IntoResponse {
    match state
        .login
        .complete_social_login(&session_id, payload)
        .await
    {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => (e.status_code(), Json(e.into_response())).into_response(),
    }
}

/// POST /api/admin/auth/idc/start
pub async fn start_idc_login(
    State(state): State<AdminState>,
    Json(payload): Json<StartIdcLoginRequest>,
) -> impl IntoResponse {
    match state.login.start_idc_login(payload).await {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => (e.status_code(), Json(e.into_response())).into_response(),
    }
}

/// POST /api/admin/auth/idc/poll/:session_id
pub async fn poll_idc_login(
    State(state): State<AdminState>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    match state.login.poll_idc_login(&session_id).await {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => (e.status_code(), Json(e.into_response())).into_response(),
    }
}

/// POST /api/admin/credentials/:id/relogin/social/start
pub async fn start_social_relogin(
    State(state): State<AdminState>,
    Path(id): Path<u64>,
    Json(payload): Json<StartSocialLoginRequest>,
) -> impl IntoResponse {
    match state.login.start_social_relogin(id, payload).await {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => (e.status_code(), Json(e.into_response())).into_response(),
    }
}

/// POST /api/admin/credentials/:id/relogin/social/poll/:session_id
pub async fn poll_social_relogin(
    State(state): State<AdminState>,
    Path((_, session_id)): Path<CredSessionPath>,
) -> impl IntoResponse {
    match state.login.poll_social_login(&session_id).await {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => (e.status_code(), Json(e.into_response())).into_response(),
    }
}

/// POST /api/admin/credentials/:id/relogin/social/complete/:session_id
pub async fn complete_social_relogin(
    State(state): State<AdminState>,
    Path((_, session_id)): Path<CredSessionPath>,
    Json(payload): Json<CompleteSocialLoginRequest>,
) -> impl IntoResponse {
    match state
        .login
        .complete_social_login(&session_id, payload)
        .await
    {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => (e.status_code(), Json(e.into_response())).into_response(),
    }
}

/// POST /api/admin/credentials/:id/relogin/idc/start
pub async fn start_idc_relogin(
    State(state): State<AdminState>,
    Path(id): Path<u64>,
    Json(payload): Json<StartIdcLoginRequest>,
) -> impl IntoResponse {
    match state.login.start_idc_relogin(id, payload).await {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => (e.status_code(), Json(e.into_response())).into_response(),
    }
}

/// POST /api/admin/credentials/:id/relogin/idc/poll/:session_id
pub async fn poll_idc_relogin(
    State(state): State<AdminState>,
    Path((_, session_id)): Path<CredSessionPath>,
) -> impl IntoResponse {
    match state.login.poll_idc_login(&session_id).await {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => (e.status_code(), Json(e.into_response())).into_response(),
    }
}
