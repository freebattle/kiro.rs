//! Anthropic API 中间件

use std::sync::Arc;
use std::time::Instant;

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Json, Response},
};
use tokio::sync::RwLock;

use crate::common::auth;
use crate::debug_log::OptionalDebugLogger;
use crate::kiro::model::available_models::RemoteModelInfo;
use crate::kiro::provider::KiroProvider;
use crate::model::config::ApiKeyEntry;
use crate::prompt_cache::PromptCacheTracker;
use crate::request_log::RequestLogStore;

use super::types::ErrorResponse;

/// 远程模型列表缓存（懒加载 + TTL）
#[derive(Clone)]
pub struct ModelsCache {
    inner: Arc<RwLock<ModelsCacheInner>>,
}

struct ModelsCacheInner {
    models: Vec<RemoteModelInfo>,
    fetched_at: Option<Instant>,
}

/// 缓存 TTL：30 分钟
const MODELS_CACHE_TTL_SECS: u64 = 30 * 60;

impl ModelsCache {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(ModelsCacheInner {
                models: Vec::new(),
                fetched_at: None,
            })),
        }
    }

    /// 获取缓存的模型列表（如果未过期）
    pub async fn get(&self) -> Option<Vec<RemoteModelInfo>> {
        let guard = self.inner.read().await;
        let fetched_at = guard.fetched_at?;
        if fetched_at.elapsed().as_secs() < MODELS_CACHE_TTL_SECS {
            Some(guard.models.clone())
        } else {
            None
        }
    }

    /// 更新缓存
    pub async fn set(&self, models: Vec<RemoteModelInfo>) {
        let mut guard = self.inner.write().await;
        guard.models = models;
        guard.fetched_at = Some(Instant::now());
    }
}

/// 调用者身份（通过请求扩展传递）
#[derive(Clone, Debug)]
pub struct CallerIdentity {
    pub name: String,
}

/// 应用共享状态
#[derive(Clone)]
pub struct AppState {
    /// 主 API 密钥
    pub api_key: String,
    /// 多 API Key 列表（可选）
    pub api_keys: Vec<ApiKeyEntry>,
    /// Kiro Provider（可选，用于实际 API 调用）
    /// 内部使用 MultiTokenManager，已支持线程安全的多凭据管理
    pub kiro_provider: Option<Arc<KiroProvider>>,
    /// 是否开启非流式响应的 thinking 块提取
    pub extract_thinking: bool,
    /// 请求记录存储
    pub request_log: RequestLogStore,
    /// Prompt Cache 模拟追踪器
    pub prompt_cache: Arc<PromptCacheTracker>,
    /// 调试日志记录器
    pub debug_logger: OptionalDebugLogger,
    /// 远程模型列表缓存
    pub models_cache: ModelsCache,
    /// 是否在模型列表中包含开源模型
    pub include_open_source_models: bool,
}

impl AppState {
    /// 创建新的应用状态
    pub fn new(
        api_key: impl Into<String>,
        extract_thinking: bool,
        request_log: RequestLogStore,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            api_keys: Vec::new(),
            kiro_provider: None,
            extract_thinking,
            request_log,
            prompt_cache: Arc::new(PromptCacheTracker::new()),
            debug_logger: OptionalDebugLogger::none(),
            models_cache: ModelsCache::new(),
            include_open_source_models: false,
        }
    }

    /// 设置是否包含开源模型
    pub fn with_include_open_source_models(mut self, include: bool) -> Self {
        self.include_open_source_models = include;
        self
    }

    /// 设置多 API Key 列表
    pub fn with_api_keys(mut self, api_keys: Vec<ApiKeyEntry>) -> Self {
        self.api_keys = api_keys;
        self
    }

    /// 设置调试日志记录器
    pub fn with_debug_logger(mut self, logger: OptionalDebugLogger) -> Self {
        self.debug_logger = logger;
        self
    }

    /// 设置 KiroProvider
    pub fn with_kiro_provider(mut self, provider: KiroProvider) -> Self {
        self.kiro_provider = Some(Arc::new(provider));
        self
    }
}

/// API Key 认证中间件
pub async fn auth_middleware(
    State(state): State<AppState>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let key = match auth::extract_api_key(&request) {
        Some(k) => k,
        None => {
            let error = ErrorResponse::authentication_error();
            return (StatusCode::UNAUTHORIZED, Json(error)).into_response();
        }
    };

    // 先检查多 key 列表
    if !state.api_keys.is_empty() {
        for entry in &state.api_keys {
            if auth::constant_time_eq(&key, &entry.key) {
                request.extensions_mut().insert(CallerIdentity {
                    name: entry.name.clone(),
                });
                return next.run(request).await;
            }
        }
    }

    // 回退到主 api_key
    if auth::constant_time_eq(&key, &state.api_key) {
        return next.run(request).await;
    }

    let error = ErrorResponse::authentication_error();
    (StatusCode::UNAUTHORIZED, Json(error)).into_response()
}

/// CORS 中间件层
///
/// **安全说明**：当前配置允许所有来源（Any），这是为了支持公开 API 服务。
/// 如果需要更严格的安全控制，请根据实际需求配置具体的允许来源、方法和头信息。
///
/// # 配置说明
/// - `allow_origin(Any)`: 允许任何来源的请求
/// - `allow_methods(Any)`: 允许任何 HTTP 方法
/// - `allow_headers(Any)`: 允许任何请求头
pub fn cors_layer() -> tower_http::cors::CorsLayer {
    use tower_http::cors::{Any, CorsLayer};

    CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any)
}
