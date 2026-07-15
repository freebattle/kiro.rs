use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// 默认 AWS / Kiro 区域（程序内常量，不暴露给用户配置）
pub const DEFAULT_REGION: &str = "us-east-1";

/// 协议指纹：Kiro IDE 版本（User-Agent / 协议对齐）
pub const KIRO_VERSION: &str = "1.0.138";

/// 协议指纹：IDE User-Agent 中的系统标识
pub const SYSTEM_VERSION: &str = "darwin#24.6.0";

/// 协议指纹：IDE User-Agent 中的 Node 版本
pub const NODE_VERSION: &str = "22.22.0";

/// 多 API Key 配置项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyEntry {
    pub key: String,
    pub name: String,
}

/// KNA 应用配置（仅用户真正需要关心的项）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default = "default_host")]
    pub host: String,

    #[serde(default = "default_port")]
    pub port: u16,

    #[serde(default)]
    pub api_key: Option<String>,

    /// 多 API Key 配置（可选，用于区分不同调用者）
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub api_keys: Vec<ApiKeyEntry>,

    /// HTTP 代理地址（可选）
    /// 支持格式: http://host:port, https://host:port, socks5://host:port
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy_url: Option<String>,

    /// 代理认证用户名（可选）
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy_username: Option<String>,

    /// 代理认证密码（可选）
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy_password: Option<String>,

    /// Admin API 密钥（可选，启用 Admin API 功能）
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admin_api_key: Option<String>,

    /// 负载均衡模式（"priority" 或 "balanced"）
    #[serde(default = "default_load_balancing_mode")]
    pub load_balancing_mode: String,

    /// 调试日志目录（设置后开启请求报文记录，不设置则关闭）
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug_log_dir: Option<String>,

    /// 默认端点名称（凭据未显式指定 endpoint 时使用，默认 "ide"）
    #[serde(default = "default_endpoint")]
    pub default_endpoint: String,

    /// 模型列表是否包含开源模型（默认 false，仅返回 Claude / GPT 等默认暴露模型）
    #[serde(default)]
    pub include_open_source_models: bool,

    /// 配置文件路径（运行时元数据，不写入 JSON）
    #[serde(skip)]
    config_path: Option<PathBuf>,
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    8080
}

fn default_load_balancing_mode() -> String {
    "priority".to_string()
}

fn default_endpoint() -> String {
    crate::kiro::endpoint::ide::IDE_ENDPOINT_NAME.to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            api_key: None,
            api_keys: Vec::new(),
            proxy_url: None,
            proxy_username: None,
            proxy_password: None,
            admin_api_key: None,
            load_balancing_mode: default_load_balancing_mode(),
            debug_log_dir: None,
            default_endpoint: default_endpoint(),
            include_open_source_models: false,
            config_path: None,
        }
    }
}

impl Config {
    /// 获取默认配置文件路径
    pub fn default_config_path() -> &'static str {
        "config.json"
    }

    /// 从文件加载配置
    ///
    /// 未知字段会被忽略（便于清理历史配置后平滑过渡）。
    pub fn load<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            let mut config = Self::default();
            config.config_path = Some(path.to_path_buf());
            return Ok(config);
        }

        let content = fs::read_to_string(path)?;
        let mut config: Config = serde_json::from_str(&content)?;
        config.config_path = Some(path.to_path_buf());
        Ok(config)
    }

    /// 获取配置文件路径（如果有）
    pub fn config_path(&self) -> Option<&Path> {
        self.config_path.as_deref()
    }

    /// 将当前配置写回原始配置文件
    pub fn save(&self) -> anyhow::Result<()> {
        let path = self
            .config_path
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("配置文件路径未知，无法保存配置"))?;

        let content = serde_json::to_string_pretty(self).context("序列化配置失败")?;
        fs::write(path, content)
            .with_context(|| format!("写入配置文件失败: {}", path.display()))?;
        Ok(())
    }
}
