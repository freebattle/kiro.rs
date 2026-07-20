//! 登录授权路由（挂到 Admin 鉴权之后）

use axum::{Router, routing::post};

use crate::admin::middleware::AdminState;
use crate::login::handlers::{
    complete_social_login, complete_social_relogin, poll_idc_login, poll_idc_relogin,
    poll_social_login, poll_social_relogin, start_idc_login, start_idc_relogin, start_social_login,
    start_social_relogin,
};

/// 返回需要 Admin API Key 的登录路由片段（无 state，由调用方 with_state）
pub fn login_routes() -> Router<AdminState> {
    Router::new()
        .route("/auth/social/start", post(start_social_login))
        .route("/auth/social/poll/{session_id}", post(poll_social_login))
        .route(
            "/auth/social/complete/{session_id}",
            post(complete_social_login),
        )
        .route("/auth/idc/start", post(start_idc_login))
        .route("/auth/idc/poll/{session_id}", post(poll_idc_login))
        .route(
            "/credentials/{id}/relogin/social/start",
            post(start_social_relogin),
        )
        .route(
            "/credentials/{id}/relogin/social/poll/{session_id}",
            post(poll_social_relogin),
        )
        .route(
            "/credentials/{id}/relogin/social/complete/{session_id}",
            post(complete_social_relogin),
        )
        .route(
            "/credentials/{id}/relogin/idc/start",
            post(start_idc_relogin),
        )
        .route(
            "/credentials/{id}/relogin/idc/poll/{session_id}",
            post(poll_idc_relogin),
        )
}
