use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::anthropic::types::{Message, SystemMessage};

const CACHE_TTL: Duration = Duration::from_secs(300);
const MAX_ENTRIES: usize = 1024;

struct CacheEntry {
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

    pub fn compute_and_update(&self, session_fingerprint: u64, input_tokens: i32) -> i32 {
        let now = Instant::now();
        let mut entries = self.entries.lock();

        let cache_read_tokens = if let Some(entry) = entries.get(&session_fingerprint) {
            if now.duration_since(entry.last_seen) < CACHE_TTL {
                tracing::debug!(
                    fingerprint = session_fingerprint,
                    last_input = entry.last_input_tokens,
                    current_input = input_tokens,
                    "prompt cache HIT"
                );
                entry.last_input_tokens
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

        entries.insert(
            session_fingerprint,
            CacheEntry {
                last_input_tokens: input_tokens,
                last_seen: now,
            },
        );

        if entries.len() > MAX_ENTRIES {
            Self::evict_expired(&mut entries, now);
        }

        cache_read_tokens
    }

    pub fn update_actual_tokens(&self, session_fingerprint: u64, actual_input_tokens: i32) {
        let mut entries = self.entries.lock();
        if let Some(entry) = entries.get_mut(&session_fingerprint) {
            tracing::debug!(
                fingerprint = session_fingerprint,
                estimated = entry.last_input_tokens,
                actual = actual_input_tokens,
                "updating cache entry with actual tokens"
            );
            entry.last_input_tokens = actual_input_tokens;
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
