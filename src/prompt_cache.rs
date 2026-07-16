use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::anthropic::types::{Message, SystemMessage};

const CACHE_TTL: Duration = Duration::from_secs(300);
const MAX_ENTRIES: usize = 1024;

struct CacheEntry {
    /// 上一轮最终确认的 input tokens（优先 contextUsage 实际值）
    last_input_tokens: i32,
    last_seen: Instant,
}

pub struct PromptCacheTracker {
    entries: Arc<Mutex<HashMap<u64, CacheEntry>>>,
}

impl PromptCacheTracker {
    pub fn new() -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 读取本轮可模拟的 cache_read，并刷新 TTL。
    ///
    /// **不会**把当前估算 input 写入基线。
    /// 基线只在 [`update_actual_tokens`] 用本轮最终 input 更新。
    ///
    /// 注意：返回值是「上一轮最终 input」作为候选缓存命中量，**不要**用请求前的
    /// 本地估算值去 clamp。本地估算常远小于 Kiro `contextUsage` 反算的最终 input，
    /// 过早 clamp 会把 cache 卡在 ~1k，日志里「输入」= final - cache 被撑到数万。
    /// 最终上报时应再用 `cache.min(final_input)` 收口。
    pub fn compute_and_update(&self, session_fingerprint: u64, input_tokens: i32) -> i32 {
        let now = Instant::now();
        let mut entries = self.entries.lock();

        let cache_read_tokens = if let Some(entry) = entries.get_mut(&session_fingerprint) {
            if now.duration_since(entry.last_seen) < CACHE_TTL {
                let hit = entry.last_input_tokens.max(0);
                entry.last_seen = now;
                tracing::debug!(
                    fingerprint = session_fingerprint,
                    last_input = entry.last_input_tokens,
                    current_input_estimate = input_tokens,
                    cache_read_candidate = hit,
                    "prompt cache HIT"
                );
                hit
            } else {
                tracing::debug!(
                    fingerprint = session_fingerprint,
                    input_tokens = input_tokens,
                    "prompt cache EXPIRED"
                );
                0
            }
        } else {
            tracing::debug!(
                fingerprint = session_fingerprint,
                input_tokens = input_tokens,
                entries_count = entries.len(),
                "prompt cache MISS (new fingerprint)"
            );
            0
        };

        if entries.len() > MAX_ENTRIES {
            Self::evict_expired(&mut entries, now);
        }

        cache_read_tokens
    }

    /// 用本轮最终 input tokens 更新会话缓存基线（供下一轮计算 cache_read）。
    ///
    /// 应在拿到 contextUsage 实际值或确定使用估算值后调用。
    pub fn update_actual_tokens(&self, session_fingerprint: u64, actual_input_tokens: i32) {
        let now = Instant::now();
        let actual = actual_input_tokens.max(0);
        let mut entries = self.entries.lock();

        if let Some(entry) = entries.get_mut(&session_fingerprint) {
            tracing::debug!(
                fingerprint = session_fingerprint,
                previous = entry.last_input_tokens,
                actual = actual,
                "finalizing prompt cache baseline"
            );
            entry.last_input_tokens = actual;
            entry.last_seen = now;
        } else {
            tracing::debug!(
                fingerprint = session_fingerprint,
                actual = actual,
                "creating prompt cache baseline"
            );
            entries.insert(
                session_fingerprint,
                CacheEntry {
                    last_input_tokens: actual,
                    last_seen: now,
                },
            );
        }

        if entries.len() > MAX_ENTRIES {
            Self::evict_expired(&mut entries, now);
        }
    }

    fn evict_expired(entries: &mut HashMap<u64, CacheEntry>, now: Instant) {
        entries.retain(|_, entry| now.duration_since(entry.last_seen) < CACHE_TTL);
        if entries.len() > MAX_ENTRIES {
            if let Some(oldest_key) = entries
                .iter()
                .min_by_key(|(_, e)| e.last_seen)
                .map(|(k, _)| *k)
            {
                entries.remove(&oldest_key);
            }
        }
    }
}

pub fn compute_session_fingerprint(
    metadata_user_id: Option<&str>,
    system: Option<&Vec<SystemMessage>>,
    messages: &[Message],
    model: &str,
) -> u64 {
    let mut hasher = DefaultHasher::new();

    // 模型参与 fingerprint，区分不同模型的缓存
    model.hash(&mut hasher);

    if let Some(user_id) = metadata_user_id {
        if let Some(session_part) = extract_session_id(user_id) {
            session_part.hash(&mut hasher);
            let fp = hasher.finish();
            tracing::debug!(
                model = model,
                session_part = &session_part[..session_part.len().min(40)],
                fingerprint = fp,
                "fingerprint from session_id"
            );
            return fp;
        }
    }

    if let Some(sys) = system {
        for msg in sys {
            msg.text.hash(&mut hasher);
        }
    }
    if let Some(first_msg) = messages.first() {
        first_msg.role.hash(&mut hasher);
        let content_str = first_msg.content.to_string();
        content_str.hash(&mut hasher);
    }

    hasher.finish()
}

fn extract_session_id(user_id: &str) -> Option<&str> {
    user_id.find("session_").map(|pos| &user_id[pos..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_uses_finalized_baseline_not_estimate() {
        let tracker = PromptCacheTracker::new();
        let fp = 42u64;

        // 首轮：无基线
        assert_eq!(tracker.compute_and_update(fp, 20_000), 0);
        // 最终实际 input 小于估算
        tracker.update_actual_tokens(fp, 16_931);

        // 次轮：cache 候选 = 上一轮最终值（即使本轮估算不同也不改）
        let cache = tracker.compute_and_update(fp, 20_500);
        assert_eq!(cache, 16_931);
        tracker.update_actual_tokens(fp, 17_028);

        // 三轮
        let cache = tracker.compute_and_update(fp, 21_000);
        assert_eq!(cache, 17_028);
        assert!(21_000 - cache > 0);
    }

    #[test]
    fn cache_candidate_not_clamped_by_low_estimate() {
        let tracker = PromptCacheTracker::new();
        let fp = 7u64;
        // 上一轮 contextUsage 最终 15k
        tracker.update_actual_tokens(fp, 15_000);
        // 本轮本地估算只有 1.1k（常见低估），候选 cache 仍应是 15k
        // 上报时再 min(15k, final_input)
        assert_eq!(tracker.compute_and_update(fp, 1_100), 15_000);
    }

    #[test]
    fn finalize_clamp_handles_input_shrink() {
        // 模拟 handlers 侧：safe_cache = candidate.min(final)
        let tracker = PromptCacheTracker::new();
        let fp = 8u64;
        tracker.update_actual_tokens(fp, 10_000);
        let candidate = tracker.compute_and_update(fp, 1_000);
        assert_eq!(candidate, 10_000);
        let final_input = 8_000;
        assert_eq!(candidate.min(final_input), 8_000);
    }

    #[test]
    fn expired_entry_is_miss() {
        let tracker = PromptCacheTracker::new();
        let fp = 9u64;
        {
            let mut entries = tracker.entries.lock();
            entries.insert(
                fp,
                CacheEntry {
                    last_input_tokens: 12_000,
                    last_seen: Instant::now() - Duration::from_secs(400),
                },
            );
        }
        assert_eq!(tracker.compute_and_update(fp, 13_000), 0);
    }
}
