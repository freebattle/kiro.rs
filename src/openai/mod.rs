//! OpenAI Responses API 兼容层
//!
//! 端点：
//! - `POST /v1/responses`（stream / non-stream）
//!
//! 内部复用 Anthropic converter → Kiro upstream。

mod converter;
mod handlers;
mod history;
mod input;
mod store;
pub mod types;

pub use handlers::post_responses;
pub use store::ResponseStore;
