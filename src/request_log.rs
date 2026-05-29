//! 请求记录模块
//!
//! 内存环形缓冲区存储请求记录，仅保留当天数据，异步写入不阻塞核心流程。

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::RwLock;
use serde::Serialize;

use crate::usage_stats::UsageStatsStore;

const MAX_RECORDS: usize = 10_000;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestRecord {
    pub model: String,
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub cache_read_tokens: i32,
    /// 首字耗时（毫秒），仅流式请求有值
    pub ttft_ms: Option<u64>,
    /// 请求总耗时（毫秒）
    pub duration_ms: u64,
    /// 请求时间戳（Unix 毫秒）
    pub timestamp: u64,
    pub stream: bool,
    pub credential_id: Option<u64>,
    pub success: bool,
    /// 消耗的 credits（从 meteringEvent 累加）
    pub credits: f64,
    /// 调用者名称（多 API Key 时标识来源）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caller: Option<String>,
    /// 思考等级（adaptive 模式下的 effort：high / medium / low）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_effort: Option<String>,
}

struct Inner {
    records: VecDeque<RequestRecord>,
    usage_stats: Option<UsageStatsStore>,
}

#[derive(Clone)]
pub struct RequestLogStore {
    inner: Arc<RwLock<Inner>>,
}

impl RequestLogStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(Inner {
                records: VecDeque::with_capacity(MAX_RECORDS),
                usage_stats: None,
            })),
        }
    }

    pub fn with_usage_stats(self, usage_stats: UsageStatsStore) -> Self {
        self.inner.write().usage_stats = Some(usage_stats);
        self
    }

    pub fn push(&self, record: RequestRecord) {
        let mut inner = self.inner.write();
        // 记录用量统计（仅成功请求）
        if record.success {
            if let (Some(stats), Some(cred_id)) = (&inner.usage_stats, record.credential_id) {
                stats.record(
                    cred_id,
                    &record.model,
                    record.input_tokens,
                    record.output_tokens,
                    record.cache_read_tokens,
                    record.credits,
                    record.caller.as_deref(),
                );
            }
        }
        if inner.records.len() >= MAX_RECORDS {
            inner.records.pop_front();
        }
        inner.records.push_back(record);
    }

    /// 获取当天的所有记录（按时间倒序）
    pub fn get_today(&self) -> Vec<RequestRecord> {
        let today_start = today_start_ms();
        let inner = self.inner.read();
        inner
            .records
            .iter()
            .filter(|r| r.timestamp >= today_start)
            .rev()
            .cloned()
            .collect()
    }

    /// 获取当天记录，支持分页和 caller 过滤
    pub fn get_today_paged(
        &self,
        page: usize,
        page_size: usize,
        caller: Option<&str>,
    ) -> (Vec<RequestRecord>, usize) {
        let today_start = today_start_ms();
        let inner = self.inner.read();
        let today_records: Vec<&RequestRecord> = inner
            .records
            .iter()
            .filter(|r| r.timestamp >= today_start)
            .filter(|r| match caller {
                Some(c) => r.caller.as_deref() == Some(c),
                None => true,
            })
            .collect();
        let total = today_records.len();
        let start = total.saturating_sub(page * page_size);
        let end = total.saturating_sub((page - 1) * page_size);
        let records: Vec<RequestRecord> = today_records[start..end]
            .iter()
            .rev()
            .map(|r| (*r).clone())
            .collect();
        (records, total)
    }

    /// 清理非当天数据
    pub fn cleanup_old(&self) {
        let today_start = today_start_ms();
        let mut inner = self.inner.write();
        while let Some(front) = inner.records.front() {
            if front.timestamp < today_start {
                inner.records.pop_front();
            } else {
                break;
            }
        }
    }

    /// 获取当天统计摘要
    pub fn get_today_stats(&self) -> RequestStats {
        let today_start = today_start_ms();
        let inner = self.inner.read();
        let today: Vec<&RequestRecord> = inner
            .records
            .iter()
            .filter(|r| r.timestamp >= today_start)
            .collect();

        let total = today.len() as u64;
        let success_count = today.iter().filter(|r| r.success).count() as u64;
        let total_input_tokens: i64 = today.iter().map(|r| r.input_tokens as i64).sum();
        let total_output_tokens: i64 = today.iter().map(|r| r.output_tokens as i64).sum();
        let total_cache_read_tokens: i64 = today.iter().map(|r| r.cache_read_tokens as i64).sum();
        let total_credits: f64 = today.iter().map(|r| r.credits).sum();
        let avg_duration_ms = if total > 0 {
            today.iter().map(|r| r.duration_ms).sum::<u64>() / total
        } else {
            0
        };
        let avg_ttft_ms = {
            let ttft_records: Vec<u64> = today.iter().filter_map(|r| r.ttft_ms).collect();
            if ttft_records.is_empty() {
                0
            } else {
                ttft_records.iter().sum::<u64>() / ttft_records.len() as u64
            }
        };

        RequestStats {
            total,
            success_count,
            total_input_tokens,
            total_output_tokens,
            total_cache_read_tokens,
            avg_duration_ms,
            avg_ttft_ms,
            total_credits,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestStats {
    pub total: u64,
    pub success_count: u64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cache_read_tokens: i64,
    pub avg_duration_ms: u64,
    pub avg_ttft_ms: u64,
    pub total_credits: f64,
}

fn today_start_ms() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    // 当天 00:00:00 UTC+8
    let offset_secs = 8 * 3600u64;
    let day_secs = 86400u64;
    let today_start_utc = ((now + offset_secs) / day_secs) * day_secs - offset_secs;
    today_start_utc * 1000
}
