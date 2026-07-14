//! Anthropic API 路由配置

use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware,
    routing::{get, post},
};

use crate::debug_log::OptionalDebugLogger;
use crate::kiro::provider::KiroProvider;
use crate::model::config::ApiKeyEntry;
use crate::request_log::RequestLogStore;

use super::{
    handlers::{count_tokens, get_models, post_messages},
    middleware::{AppState, auth_middleware, cors_layer},
};
use crate::openai::post_responses;

/// 请求体最大大小限制 (50MB)
const MAX_BODY_SIZE: usize = 50 * 1024 * 1024;

/// 创建带有 KiroProvider 的 Anthropic API 路由
pub fn create_router_with_provider(
    api_key: impl Into<String>,
    api_keys: Vec<ApiKeyEntry>,
    kiro_provider: Option<KiroProvider>,
    extract_thinking: bool,
    request_log: RequestLogStore,
    debug_logger: OptionalDebugLogger,
    include_open_source_models: bool,
) -> Router {
    let mut state = AppState::new(api_key, extract_thinking, request_log)
        .with_api_keys(api_keys)
        .with_debug_logger(debug_logger)
        .with_include_open_source_models(include_open_source_models);
    if let Some(provider) = kiro_provider {
        state = state.with_kiro_provider(provider);
    }

    // 需要认证的 /v1 路由
    let v1_routes = Router::new()
        .route("/models", get(get_models))
        .route("/messages", post(post_messages))
        .route("/messages/count_tokens", post(count_tokens))
        .route("/responses", post(post_responses))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    Router::new()
        .nest("/v1", v1_routes)
        .layer(cors_layer())
        .layer(DefaultBodyLimit::max(MAX_BODY_SIZE))
        .with_state(state)
}
