//! 凭据 OAuth 登录授权模块
//!
//! 与主 API 调用链路隔离：
//! - 只通过 Admin API Key 访问
//! - 只写入 MultiTokenManager 凭据池
//! - 不参与 messages/responses 请求转发
//!
//! 目录：
//! - `social` / `idc`：协议实现（PKCE / 设备码）
//! - `service`：会话编排
//! - `handlers` / `router`：HTTP 层

pub mod handlers;
pub mod idc;
pub mod router;
pub mod service;
pub mod social;
pub mod types;

pub use router::login_routes;
pub use service::LoginService;
