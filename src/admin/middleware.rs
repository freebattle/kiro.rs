//! Admin API 中间件

use std::sync::Arc;

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Json, Response},
};

use super::service::AdminService;
use super::types::AdminErrorResponse;
use crate::common::auth;
use crate::request_log::RequestLogStore;
use crate::usage_stats::UsageStatsStore;

/// Admin API 共享状态
#[derive(Clone)]
pub struct AdminState {
    /// Admin API 密钥
    pub admin_api_key: String,
    /// Admin 服务
    pub service: Arc<AdminService>,
    /// 请求记录存储
    pub request_log: RequestLogStore,
    /// 用量统计存储
    pub usage_stats: UsageStatsStore,
}

impl AdminState {
    pub fn new(admin_api_key: impl Into<String>, service: AdminService, request_log: RequestLogStore, usage_stats: UsageStatsStore) -> Self {
        Self {
            admin_api_key: admin_api_key.into(),
            service: Arc::new(service),
            request_log,
            usage_stats,
        }
    }
}

/// Admin API 认证中间件
pub async fn admin_auth_middleware(
    State(state): State<AdminState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let api_key = auth::extract_api_key(&request);

    match api_key {
        Some(key) if auth::constant_time_eq(&key, &state.admin_api_key) => next.run(request).await,
        _ => {
            let error = AdminErrorResponse::authentication_error();
            (StatusCode::UNAUTHORIZED, Json(error)).into_response()
        }
    }
}
