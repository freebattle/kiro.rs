//! Admin API 路由配置

use axum::{
    Router, middleware,
    routing::{delete, get, post},
};

use super::{
    handlers::{
        add_credential, delete_credential, force_refresh_token, get_all_credentials,
        get_credential_balance, get_load_balancing_mode, get_request_logs, get_request_stats,
        get_usage_stats, reset_failure_count, set_credential_disabled, set_credential_priority,
        set_load_balancing_mode,
    },
    middleware::{AdminState, admin_auth_middleware},
};

pub fn create_admin_router(state: AdminState) -> Router {
    Router::new()
        .route(
            "/credentials",
            get(get_all_credentials).post(add_credential),
        )
        .route("/credentials/{id}", delete(delete_credential))
        .route("/credentials/{id}/disabled", post(set_credential_disabled))
        .route("/credentials/{id}/priority", post(set_credential_priority))
        .route("/credentials/{id}/reset", post(reset_failure_count))
        .route("/credentials/{id}/refresh", post(force_refresh_token))
        .route("/credentials/{id}/balance", get(get_credential_balance))
        .route(
            "/config/load-balancing",
            get(get_load_balancing_mode).put(set_load_balancing_mode),
        )
        .route("/requests", get(get_request_logs))
        .route("/requests/stats", get(get_request_stats))
        .route("/usage-stats", get(get_usage_stats))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            admin_auth_middleware,
        ))
        .with_state(state)
}
