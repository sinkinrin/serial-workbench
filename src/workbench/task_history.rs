use super::{model::*, storage, tasks::Task, Hub};
use anyhow::{Context, Result};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::{
    fs,
    sync::{watch, Mutex},
};

pub fn task_path(root: &Path, id: &str) -> Result<PathBuf> {
    let id = uuid::Uuid::parse_str(id)?;
    Ok(root.join("tasks").join(format!("{id}.json")))
}
impl Task {
    pub fn from_details(record: TaskDetails, path: PathBuf) -> Self {
        let (cancel, _) = watch::channel(false);
        Self {
            info: Mutex::new(record.info),
            steps: record.steps,
            script: record.script,
            transcript: Mutex::new(record.transcript),
            reconstructed: record.reconstructed,
            path,
            persist_lock: Mutex::new(()),
            cancel,
        }
    }
    pub async fn snapshot(&self) -> TaskDetails {
        TaskDetails {
            info: self.info.lock().await.clone(),
            steps: self.steps.clone(),
            script: self.script.clone(),
            transcript: self.transcript.lock().await.clone(),
            reconstructed: self.reconstructed,
        }
    }
    pub async fn persist(&self) -> Result<()> {
        let _guard = self.persist_lock.lock().await;
        storage::write_json(&self.path, &self.snapshot().await).await
    }
}

pub async fn load_tasks(dir: &Path) -> Result<HashMap<String, Arc<Task>>> {
    fs::create_dir_all(dir).await?;
    let mut entries = fs::read_dir(dir).await?;
    let mut paths = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json") {
            let modified = entry
                .metadata()
                .await?
                .modified()
                .unwrap_or(std::time::UNIX_EPOCH);
            paths.push((modified, path));
        }
    }
    paths.sort_by(|a, b| b.0.cmp(&a.0));
    let mut result = HashMap::new();
    for (_, path) in paths.into_iter().take(1000) {
        let bytes = fs::read(&path).await?;
        let Ok(mut record) = serde_json::from_slice::<TaskDetails>(&bytes) else {
            tracing::warn!("Skipped unreadable task record: {}", path.display());
            continue;
        };
        if record.info.finished_at.is_none() {
            record.info.status = "interrupted".into();
            record.info.finished_at = Some(now());
            record.info.message =
                Some("Daemon stopped before completion; task was not resumed".into());
            record.transcript.finalized = true;
            record.transcript.gap = true;
            storage::write_json(&path, &record).await?;
        }
        result.insert(
            record.info.id.clone(),
            Arc::new(Task::from_details(record, path)),
        );
    }
    Ok(result)
}

impl Hub {
    pub async fn task_details(&self, id: &str) -> Result<TaskDetails> {
        if let Some(task) = self.tasks.read().await.get(id).cloned() {
            let mut record = task.snapshot().await;
            if !record.transcript.finalized {
                record.transcript = self.capture_task(&record.info).await?;
                if record.transcript.finalized {
                    *task.transcript.lock().await = record.transcript.clone();
                    task.persist().await?;
                }
            }
            return Ok(record);
        }
        let path = task_path(&self.data_dir, id)?;
        let mut record: TaskDetails =
            serde_json::from_slice(&fs::read(&path).await.context("task not found")?)?;
        if !record.transcript.finalized {
            record.transcript = self.capture_task(&record.info).await?;
            if record.transcript.finalized {
                storage::write_json(&path, &record).await?;
            }
        }
        Ok(record)
    }

    pub async fn capture_task(&self, info: &TaskInfo) -> Result<TaskTranscript> {
        let finish = info
            .finished_at
            .as_ref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok());
        let cutoff = finish.map(|t| t + chrono::Duration::seconds(1));
        let settled = cutoff.is_some_and(|t| chrono::Utc::now() >= t);
        let mut result = TaskTranscript {
            finalized: settled,
            hidden: info.sensitive,
            next_cursor: info.start_cursor,
            ..Default::default()
        };
        if info.sensitive || info.start_cursor == 0 {
            result.finalized = info.finished_at.is_some();
            return Ok(result);
        }
        let mut cursor = info.start_cursor;
        let mut disk = false;
        // A bounded receive window, not an assertion that unsolicited device logs belong to the command.
        for _ in 0..16 {
            let query = EventQuery {
                after: cursor,
                limit: Some(200),
                contains: None,
            };
            let mut page = self.events(&info.session_id, &query, disk).await?;
            if page.gap && !disk {
                disk = true;
                page = self.events(&info.session_id, &query, true).await?;
            }
            result.gap |= page.gap;
            for event in page.events {
                if let Some(end) = cutoff {
                    if chrono::DateTime::parse_from_rfc3339(&event.time).is_ok_and(|t| t > end) {
                        result.finalized = true;
                        return Ok(result);
                    }
                }
                if event.task_id.as_deref().is_some_and(|id| id != info.id)
                    && (event.direction == "tx"
                        || (event.direction == "system" && event.text == "Task started"))
                {
                    result.finalized = info.finished_at.is_some();
                    return Ok(result);
                }
                result.next_cursor = event.seq;
                if event.direction == "rx"
                    || (event.direction == "tx" && event.task_id.as_deref() == Some(&info.id))
                {
                    if result.events.len() >= 512 || result.bytes_returned + event.bytes > 65536 {
                        result.truncated = true;
                        result.finalized = info.finished_at.is_some();
                        return Ok(result);
                    }
                    result.bytes_returned += event.bytes;
                    result.events.push(event);
                }
            }
            if !page.has_more || page.next_cursor <= cursor {
                return Ok(result);
            }
            cursor = page.next_cursor;
        }
        result.truncated = true;
        result.finalized = info.finished_at.is_some();
        Ok(result)
    }
}
