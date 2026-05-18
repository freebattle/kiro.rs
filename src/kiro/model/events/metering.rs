//! 计费事件
//!
//! 处理 meteringEvent 类型的事件

use serde::Deserialize;

use crate::kiro::parser::error::ParseResult;
use crate::kiro::parser::frame::Frame;

use super::base::EventPayload;

/// 计费事件
///
/// 包含本次请求消耗的 credits
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeteringEvent {
    #[serde(default)]
    pub usage: f64,
}

impl EventPayload for MeteringEvent {
    fn from_frame(frame: &Frame) -> ParseResult<Self> {
        frame.payload_as_json()
    }
}

impl std::fmt::Display for MeteringEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.6} credits", self.usage)
    }
}
