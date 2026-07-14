//! Responses 持久化（previous_response_id 支持）

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::types::{
    ResponseOutputItem, ResponsesError, ResponsesObject, ResponsesUsage,
};

/// 默认 TTL：30 天
pub const RESPONSES_DEFAULT_TTL_SECS: u64 = 30 * 24 * 60 * 60;

#[derive(Debug)]
pub enum StoreError {
    MissingId,
    Io(std::io::Error),
    Json(serde_json::Error),
    Expired,
    NotFound,
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingId => write!(f, "response missing id"),
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Json(e) => write!(f, "json error: {e}"),
            Self::Expired => write!(f, "stored response expired"),
            Self::NotFound => write!(f, "stored response not found"),
        }
    }
}

impl std::error::Error for StoreError {}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredResponseDoc {
    id: String,
    object: String,
    created_at: i64,
    status: String,
    model: String,
    output: Vec<ResponseOutputItem>,
    usage: ResponsesUsage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_response_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    metadata: Option<std::collections::HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<ResponsesError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stored_input: Option<Value>,
    stored_at: i64,
}

/// 磁盘 response store
#[derive(Clone, Debug)]
pub struct ResponseStore {
    dir: PathBuf,
    ttl_secs: u64,
}

impl ResponseStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            ttl_secs: RESPONSES_DEFAULT_TTL_SECS,
        }
    }

    pub fn with_ttl(mut self, ttl_secs: u64) -> Self {
        self.ttl_secs = ttl_secs;
        self
    }

    #[allow(dead_code)]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn save(&self, resp: &ResponsesObject) -> Result<(), StoreError> {
        if resp.id.is_empty() {
            return Err(StoreError::MissingId);
        }
        fs::create_dir_all(&self.dir).map_err(StoreError::Io)?;

        let stored_at = resp.stored_at.unwrap_or_else(now_unix);
        let doc = StoredResponseDoc {
            id: resp.id.clone(),
            object: resp.object.clone(),
            created_at: resp.created_at,
            status: resp.status.clone(),
            model: resp.model.clone(),
            output: resp.output.clone(),
            usage: resp.usage.clone(),
            previous_response_id: resp.previous_response_id.clone(),
            metadata: resp.metadata.clone(),
            instructions: resp.instructions.clone(),
            error: resp.error.clone(),
            stored_input: resp.stored_input.clone(),
            stored_at,
        };

        let path = self.path_for(&resp.id);
        let data = serde_json::to_vec_pretty(&doc).map_err(StoreError::Json)?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, data).map_err(StoreError::Io)?;
        fs::rename(&tmp, &path).map_err(StoreError::Io)?;
        Ok(())
    }

    pub fn load(&self, id: &str) -> Result<ResponsesObject, StoreError> {
        if id.is_empty() {
            return Err(StoreError::MissingId);
        }
        let path = self.path_for(id);
        let data = fs::read(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                StoreError::NotFound
            } else {
                StoreError::Io(e)
            }
        })?;
        let doc: StoredResponseDoc = serde_json::from_slice(&data).map_err(StoreError::Json)?;
        if doc.stored_at > 0 {
            let age = now_unix().saturating_sub(doc.stored_at) as u64;
            if age > self.ttl_secs {
                let _ = fs::remove_file(&path);
                return Err(StoreError::Expired);
            }
        }
        Ok(ResponsesObject {
            id: doc.id,
            object: doc.object,
            created_at: doc.created_at,
            status: doc.status,
            model: doc.model,
            output: doc.output,
            usage: doc.usage,
            previous_response_id: doc.previous_response_id,
            metadata: doc.metadata,
            error: doc.error,
            instructions: doc.instructions,
            stored_input: doc.stored_input,
            stored_at: Some(doc.stored_at),
        })
    }

    pub fn purge_expired(&self) {
        let entries = match fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        let cutoff = now_unix().saturating_sub(self.ttl_secs as i64);
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let Ok(modified) = meta.modified() else { continue };
            let modified_unix = modified
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            if modified_unix < cutoff {
                let _ = fs::remove_file(path);
            }
        }
    }

    fn path_for(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{}.json", sanitize_response_id(id)))
    }
}

pub fn generate_response_id() -> String {
    let rand_part = uuid::Uuid::new_v4().simple().to_string();
    // 取前 24 hex 对齐 Kiro-Go 风格 resp_<12bytes hex><8 hex time>
    let head = &rand_part[..24.min(rand_part.len())];
    let t = now_unix() as u32;
    format!("resp_{head}{t:08x}")
}

pub fn generate_output_item_id(prefix: &str) -> String {
    let rand_part = uuid::Uuid::new_v4().simple().to_string();
    format!("{prefix}_{}", &rand_part[..16.min(rand_part.len())])
}

pub fn sanitize_response_id(id: &str) -> String {
    let cleaned: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    if cleaned.is_empty() {
        "invalid".to_string()
    } else {
        cleaned
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openai::types::{ResponseContentPart, ResponseOutputItem, ResponsesUsage};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_store() -> (ResponseStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "kiro-rs-responses-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        (ResponseStore::new(&dir), dir)
    }

    #[test]
    fn test_save_and_load_response() {
        let (store, dir) = tmp_store();
        let id = generate_response_id();
        let resp = ResponsesObject {
            id: id.clone(),
            object: "response".to_string(),
            created_at: now_unix(),
            status: "completed".to_string(),
            model: "gpt-5.6-luna".to_string(),
            output: vec![ResponseOutputItem {
                id: generate_output_item_id("msg"),
                item_type: "message".to_string(),
                role: Some("assistant".to_string()),
                status: Some("completed".to_string()),
                content: vec![ResponseContentPart {
                    part_type: "output_text".to_string(),
                    text: Some("hello".to_string()),
                }],
                call_id: None,
                name: None,
                arguments: None,
                input: None,
            }],
            usage: ResponsesUsage {
                input_tokens: 10,
                output_tokens: 2,
                total_tokens: 12,
            },
            previous_response_id: None,
            metadata: None,
            error: None,
            instructions: Some("be brief".to_string()),
            stored_input: Some(serde_json::json!("hello")),
            stored_at: None,
        };
        store.save(&resp).unwrap();
        let loaded = store.load(&id).unwrap();
        assert_eq!(loaded.id, id);
        assert_eq!(loaded.output[0].content[0].text.as_deref(), Some("hello"));
        assert_eq!(loaded.instructions.as_deref(), Some("be brief"));
        assert_eq!(loaded.stored_input, Some(serde_json::json!("hello")));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_load_missing() {
        let (store, dir) = tmp_store();
        let err = store.load("resp_missing").unwrap_err();
        assert!(matches!(err, StoreError::NotFound));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_expired_response() {
        let (store, dir) = tmp_store();
        let store = store.with_ttl(1);
        let id = "resp_expired_test";
        let mut resp = ResponsesObject {
            id: id.to_string(),
            object: "response".to_string(),
            created_at: now_unix() - 100,
            status: "completed".to_string(),
            model: "m".to_string(),
            output: vec![],
            usage: ResponsesUsage::default(),
            previous_response_id: None,
            metadata: None,
            error: None,
            instructions: None,
            stored_input: None,
            stored_at: Some(now_unix() - 100),
        };
        store.save(&resp).unwrap();
        // 手动把 stored_at 改得很旧
        resp.stored_at = Some(now_unix() - 10);
        store.save(&resp).unwrap();
        let err = store.load(id).unwrap_err();
        assert!(matches!(err, StoreError::Expired));
        let _ = fs::remove_dir_all(dir);
    }
}
