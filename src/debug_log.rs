use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;
use tokio::sync::mpsc;

struct LogEntry {
    filename: String,
    content: String,
}

#[derive(Clone)]
pub struct DebugLogger {
    tx: mpsc::UnboundedSender<LogEntry>,
}

impl DebugLogger {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(writer_task(dir, rx));
        Self { tx }
    }

    pub fn log_request(&self, request_id: &str, anthropic_body: &str, kiro_body: &str) {
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S%.3f");
        let filename = format!("{}_{}", timestamp, request_id);

        let _ = self.tx.send(LogEntry {
            filename: format!("{}_anthropic_req.json", filename),
            content: anthropic_body.to_string(),
        });
        let _ = self.tx.send(LogEntry {
            filename: format!("{}_kiro_req.json", filename),
            content: kiro_body.to_string(),
        });
    }
}

async fn writer_task(dir: PathBuf, mut rx: mpsc::UnboundedReceiver<LogEntry>) {
    if let Err(e) = fs::create_dir_all(&dir).await {
        tracing::error!("创建 debug_log 目录失败: {}", e);
        return;
    }
    while let Some(entry) = rx.recv().await {
        let path = dir.join(&entry.filename);
        if let Err(e) = fs::write(&path, &entry.content).await {
            tracing::warn!("写入 debug log 失败 {}: {}", entry.filename, e);
        }
    }
}

#[derive(Clone)]
pub struct OptionalDebugLogger(Option<Arc<DebugLogger>>);

impl OptionalDebugLogger {
    pub fn none() -> Self {
        Self(None)
    }

    pub fn some(logger: DebugLogger) -> Self {
        Self(Some(Arc::new(logger)))
    }

    pub fn log_request(&self, request_id: &str, anthropic_body: &str, kiro_body: &str) {
        if let Some(logger) = &self.0 {
            logger.log_request(request_id, anthropic_body, kiro_body);
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.0.is_some()
    }
}
