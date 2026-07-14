//! ListAvailableModels API 响应类型

use serde::Deserialize;

/// ListAvailableModels 响应
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct AvailableModelsResponse {
    /// 默认模型
    #[serde(default)]
    pub default_model: Option<RemoteModelInfo>,

    /// 可用模型列表
    #[serde(default)]
    pub models: Vec<RemoteModelInfo>,
}

/// 远程模型信息
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct RemoteModelInfo {
    /// 模型 ID（如 "claude-opus-4.8"）
    pub model_id: String,

    /// 模型显示名称
    #[serde(default)]
    pub model_name: Option<String>,

    /// 模型描述
    #[serde(default)]
    pub description: Option<String>,

    /// 支持的输入类型（如 ["TEXT", "IMAGE"]）
    #[serde(default)]
    pub supported_input_types: Vec<String>,

    /// 费率倍数
    #[serde(default)]
    pub rate_multiplier: Option<f64>,

    /// 费率单位
    #[serde(default)]
    pub rate_unit: Option<String>,

    /// Prompt Caching 能力
    #[serde(default)]
    pub prompt_caching: Option<PromptCaching>,

    /// 附加请求字段 Schema
    #[serde(default)]
    pub additional_model_request_fields_schema: Option<serde_json::Value>,

    /// Token 限制
    #[serde(default)]
    pub token_limits: Option<TokenLimits>,
}

/// Token 限制
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct TokenLimits {
    pub max_input_tokens: Option<i64>,
    pub max_output_tokens: Option<i64>,
}

/// Prompt Caching 配置
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct PromptCaching {
    #[serde(default)]
    pub supports_prompt_caching: Option<bool>,
    #[serde(default)]
    pub minimum_tokens_per_cache_checkpoint: Option<i64>,
    #[serde(default)]
    pub maximum_cache_checkpoints_per_request: Option<i64>,
}
