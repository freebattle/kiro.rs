//! 月度用量统计模块
//!
//! 按自然月、凭据、模型维度累积请求次数和 token 用量。
//! 内存中累积，后台定时刷盘到 JSON 文件。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelUsage {
    pub requests: u64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    #[serde(default)]
    pub cache_read_tokens: i64,
    #[serde(default)]
    pub credits: f64,
}

/// credential_id -> model -> usage
pub type CredentialUsageMap = HashMap<u64, HashMap<String, ModelUsage>>;

/// caller_name -> model -> usage
pub type CallerUsageMap = HashMap<String, HashMap<String, ModelUsage>>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MonthlyUsage {
    pub month: String,
    pub credentials: CredentialUsageMap,
    #[serde(default)]
    pub callers: CallerUsageMap,
}

#[derive(Clone)]
pub struct UsageStatsStore {
    inner: Arc<Mutex<MonthlyUsage>>,
    data_dir: PathBuf,
}

impl UsageStatsStore {
    pub fn new(data_dir: impl AsRef<Path>) -> Self {
        let data_dir = data_dir.as_ref().to_path_buf();
        let month = current_month();
        let usage = load_from_file(&data_dir, &month);
        Self {
            inner: Arc::new(Mutex::new(usage)),
            data_dir,
        }
    }

    pub fn record(&self, credential_id: u64, model: &str, input_tokens: i32, output_tokens: i32, cache_read_tokens: i32, credits: f64, caller: Option<&str>) {
        let month = current_month();
        let mut data = self.inner.lock();
        if data.month != month {
            flush_to_file(&self.data_dir, &data);
            *data = load_from_file(&self.data_dir, &month);
        }
        let model_map = data.credentials.entry(credential_id).or_default();
        let usage = model_map.entry(model.to_string()).or_default();
        usage.requests += 1;
        usage.input_tokens += input_tokens as i64;
        usage.output_tokens += output_tokens as i64;
        usage.cache_read_tokens += cache_read_tokens as i64;
        usage.credits += credits;

        if let Some(name) = caller {
            let caller_map = data.callers.entry(name.to_string()).or_default();
            let caller_usage = caller_map.entry(model.to_string()).or_default();
            caller_usage.requests += 1;
            caller_usage.input_tokens += input_tokens as i64;
            caller_usage.output_tokens += output_tokens as i64;
            caller_usage.cache_read_tokens += cache_read_tokens as i64;
            caller_usage.credits += credits;
        }
    }

    pub fn get_month(&self, month: &str) -> MonthlyUsage {
        let data = self.inner.lock();
        if data.month == month {
            return data.clone();
        }
        drop(data);
        load_from_file(&self.data_dir, month)
    }

    pub fn flush(&self) {
        let data = self.inner.lock();
        flush_to_file(&self.data_dir, &data);
    }

    pub fn spawn_flush_task(&self) {
        let store = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                store.flush();
            }
        });
    }
}

fn current_month() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    // UTC+8
    let ts = now + 8 * 3600;
    let days = ts / 86400;
    let (year, month, _) = days_to_ymd(days);
    format!("{:04}-{:02}", year, month)
}

fn days_to_ymd(days_since_epoch: u64) -> (u64, u64, u64) {
    // Civil days algorithm from Howard Hinnant
    let z = days_since_epoch + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

fn file_path(data_dir: &Path, month: &str) -> PathBuf {
    data_dir.join(format!("usage_{}.json", month))
}

fn load_from_file(data_dir: &Path, month: &str) -> MonthlyUsage {
    let path = file_path(data_dir, month);
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or(MonthlyUsage {
            month: month.to_string(),
            credentials: HashMap::new(),
            callers: HashMap::new(),
        }),
        Err(_) => MonthlyUsage {
            month: month.to_string(),
            credentials: HashMap::new(),
            callers: HashMap::new(),
        },
    }
}

fn flush_to_file(data_dir: &Path, data: &MonthlyUsage) {
    if data.month.is_empty() {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(data_dir) {
        tracing::warn!("创建用量统计目录失败: {}", e);
        return;
    }
    let path = file_path(data_dir, &data.month);
    match serde_json::to_string_pretty(data) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                tracing::warn!("写入用量统计文件失败: {}", e);
            }
        }
        Err(e) => {
            tracing::warn!("序列化用量统计失败: {}", e);
        }
    }
}
