//! Background serial recording with file rotation and an in-memory tail buffer.

use crate::serial::{LocalSerialError, SerialConnection};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::fs::{self, File, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::sync::{oneshot, Mutex, RwLock};
use tokio::task::JoinHandle;

#[derive(Debug, Error)]
pub enum RecordingError {
    #[error("recording already exists for connection {0}")]
    AlreadyExists(String),
    #[error("recording not found for connection {0}")]
    NotFound(String),
    #[error("invalid recording configuration: {0}")]
    InvalidConfig(String),
    #[error("recording I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("recording task failed: {0}")]
    Task(String),
}

pub type RecordingResult<T> = Result<T, RecordingError>;

#[derive(Debug, Clone)]
pub struct RecordingOptions {
    pub path: PathBuf,
    pub max_file_size_bytes: u64,
    pub max_files: usize,
    pub read_chunk_size: usize,
    pub read_timeout_ms: u64,
    pub ring_buffer_bytes: usize,
    pub flush_interval_ms: u64,
}

impl RecordingOptions {
    pub fn validate(&self) -> RecordingResult<()> {
        if self.path.as_os_str().is_empty() {
            return Err(RecordingError::InvalidConfig(
                "path must not be empty".to_string(),
            ));
        }
        if self.max_file_size_bytes == 0 {
            return Err(RecordingError::InvalidConfig(
                "max_file_size_bytes must be greater than zero".to_string(),
            ));
        }
        if self.max_files == 0 {
            return Err(RecordingError::InvalidConfig(
                "max_files must be greater than zero".to_string(),
            ));
        }
        if self.read_chunk_size == 0 || self.read_chunk_size > 1024 * 1024 {
            return Err(RecordingError::InvalidConfig(
                "read_chunk_size must be between 1 and 1048576 bytes".to_string(),
            ));
        }
        if self.read_timeout_ms == 0 {
            return Err(RecordingError::InvalidConfig(
                "read_timeout_ms must be greater than zero".to_string(),
            ));
        }
        if self.ring_buffer_bytes == 0 || self.ring_buffer_bytes > 16 * 1024 * 1024 {
            return Err(RecordingError::InvalidConfig(
                "ring_buffer_bytes must be between 1 and 16777216 bytes".to_string(),
            ));
        }
        if self.flush_interval_ms == 0 {
            return Err(RecordingError::InvalidConfig(
                "flush_interval_ms must be greater than zero".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RecordingStatus {
    pub connection_id: String,
    pub path: String,
    pub active: bool,
    pub started_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stopped_at: Option<DateTime<Utc>>,
    pub bytes_written: u64,
    pub chunks_written: u64,
    pub current_file_size: u64,
    pub rotations: u64,
    pub ring_buffer_bytes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecordingTail {
    pub connection_id: String,
    pub bytes: Vec<u8>,
    pub buffered_bytes: usize,
}

#[derive(Debug)]
struct RecordingState {
    active: bool,
    started_at: DateTime<Utc>,
    stopped_at: Option<DateTime<Utc>>,
    bytes_written: u64,
    chunks_written: u64,
    current_file_size: u64,
    rotations: u64,
    last_error: Option<String>,
}

#[derive(Debug)]
struct RecordingSession {
    connection_id: String,
    options: RecordingOptions,
    state: Mutex<RecordingState>,
    ring: Mutex<VecDeque<u8>>,
    cancel: Mutex<Option<oneshot::Sender<()>>>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl RecordingSession {
    async fn status(&self) -> RecordingStatus {
        let state = self.state.lock().await;
        let ring_buffer_bytes = self.ring.lock().await.len();
        RecordingStatus {
            connection_id: self.connection_id.clone(),
            path: self.options.path.to_string_lossy().into_owned(),
            active: state.active,
            started_at: state.started_at,
            stopped_at: state.stopped_at,
            bytes_written: state.bytes_written,
            chunks_written: state.chunks_written,
            current_file_size: state.current_file_size,
            rotations: state.rotations,
            ring_buffer_bytes,
            last_error: state.last_error.clone(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RecordingManager {
    sessions: Arc<RwLock<HashMap<String, Arc<RecordingSession>>>>,
}

impl RecordingManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn start(
        &self,
        connection_id: String,
        connection: Arc<SerialConnection>,
        options: RecordingOptions,
    ) -> RecordingResult<RecordingStatus> {
        options.validate()?;

        {
            let sessions = self.sessions.read().await;
            if sessions.contains_key(&connection_id) {
                return Err(RecordingError::AlreadyExists(connection_id));
            }
        }

        if let Some(parent) = options.path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).await?;
            }
        }

        let writer = RotatingFileWriter::open(
            options.path.clone(),
            options.max_file_size_bytes,
            options.max_files,
        )
        .await?;
        let current_file_size = writer.current_size;
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let session = Arc::new(RecordingSession {
            connection_id: connection_id.clone(),
            options: options.clone(),
            state: Mutex::new(RecordingState {
                active: true,
                started_at: Utc::now(),
                stopped_at: None,
                bytes_written: 0,
                chunks_written: 0,
                current_file_size,
                rotations: 0,
                last_error: None,
            }),
            ring: Mutex::new(VecDeque::with_capacity(
                options.ring_buffer_bytes.min(1024 * 1024),
            )),
            cancel: Mutex::new(Some(cancel_tx)),
            task: Mutex::new(None),
        });

        {
            let mut sessions = self.sessions.write().await;
            if sessions.contains_key(&connection_id) {
                return Err(RecordingError::AlreadyExists(connection_id));
            }
            sessions.insert(connection_id.clone(), Arc::clone(&session));
        }

        let task_session = Arc::clone(&session);
        let task = tokio::spawn(async move {
            record_loop(task_session, connection, writer, cancel_rx).await;
        });
        *session.task.lock().await = Some(task);

        Ok(session.status().await)
    }

    pub async fn stop(&self, connection_id: &str) -> RecordingResult<RecordingStatus> {
        let session = {
            let sessions = self.sessions.read().await;
            sessions
                .get(connection_id)
                .cloned()
                .ok_or_else(|| RecordingError::NotFound(connection_id.to_string()))?
        };

        if let Some(cancel) = session.cancel.lock().await.take() {
            let _ = cancel.send(());
        }
        let task_result = if let Some(task) = session.task.lock().await.take() {
            task.await
                .map_err(|error| RecordingError::Task(error.to_string()))
        } else {
            Ok(())
        };

        if let Err(error) = &task_result {
            let mut state = session.state.lock().await;
            state.active = false;
            state.stopped_at.get_or_insert_with(Utc::now);
            state.last_error = Some(error.to_string());
        }

        let status = session.status().await;
        self.sessions.write().await.remove(connection_id);
        task_result?;
        Ok(status)
    }

    pub async fn stop_if_exists(
        &self,
        connection_id: &str,
    ) -> RecordingResult<Option<RecordingStatus>> {
        if self.sessions.read().await.contains_key(connection_id) {
            self.stop(connection_id).await.map(Some)
        } else {
            Ok(None)
        }
    }

    pub async fn stop_all(&self) -> Vec<RecordingStatus> {
        let connection_ids: Vec<String> = self.sessions.read().await.keys().cloned().collect();
        let mut reports = Vec::with_capacity(connection_ids.len());
        for connection_id in connection_ids {
            if let Ok(report) = self.stop(&connection_id).await {
                reports.push(report);
            }
        }
        reports
    }

    pub async fn status(
        &self,
        connection_id: Option<&str>,
    ) -> RecordingResult<Vec<RecordingStatus>> {
        let sessions: Vec<Arc<RecordingSession>> = {
            let sessions = self.sessions.read().await;
            if let Some(connection_id) = connection_id {
                vec![sessions
                    .get(connection_id)
                    .cloned()
                    .ok_or_else(|| RecordingError::NotFound(connection_id.to_string()))?]
            } else {
                sessions.values().cloned().collect()
            }
        };

        let mut statuses = Vec::with_capacity(sessions.len());
        for session in sessions {
            statuses.push(session.status().await);
        }
        Ok(statuses)
    }

    pub async fn tail(
        &self,
        connection_id: &str,
        max_bytes: usize,
    ) -> RecordingResult<RecordingTail> {
        if max_bytes == 0 || max_bytes > 16 * 1024 * 1024 {
            return Err(RecordingError::InvalidConfig(
                "max_bytes must be between 1 and 16777216 bytes".to_string(),
            ));
        }
        let session = {
            let sessions = self.sessions.read().await;
            sessions
                .get(connection_id)
                .cloned()
                .ok_or_else(|| RecordingError::NotFound(connection_id.to_string()))?
        };
        let ring = session.ring.lock().await;
        let buffered_bytes = ring.len();
        let start = buffered_bytes.saturating_sub(max_bytes);
        let bytes = ring.iter().skip(start).copied().collect();
        Ok(RecordingTail {
            connection_id: connection_id.to_string(),
            bytes,
            buffered_bytes,
        })
    }

    pub async fn is_active(&self, connection_id: &str) -> bool {
        let session = self.sessions.read().await.get(connection_id).cloned();
        match session {
            Some(session) => session.state.lock().await.active,
            None => false,
        }
    }
}

async fn record_loop(
    session: Arc<RecordingSession>,
    connection: Arc<SerialConnection>,
    mut writer: RotatingFileWriter,
    mut cancel_rx: oneshot::Receiver<()>,
) {
    let mut buffer = vec![0_u8; session.options.read_chunk_size];
    let mut last_flush = tokio::time::Instant::now();

    loop {
        let read_result = tokio::select! {
            _ = &mut cancel_rx => break,
            result = connection.read(&mut buffer, Some(session.options.read_timeout_ms)) => result,
        };

        match read_result {
            Ok(0) => {
                set_recording_error(&session, "serial stream returned zero bytes".to_string())
                    .await;
                break;
            }
            Ok(bytes_read) => {
                let chunk = &buffer[..bytes_read];
                if let Err(error) = writer.write_all(chunk).await {
                    set_recording_error(&session, error.to_string()).await;
                    break;
                }
                let mut ring = session.ring.lock().await;
                append_ring(&mut ring, chunk, session.options.ring_buffer_bytes);
                drop(ring);
                {
                    let mut state = session.state.lock().await;
                    state.bytes_written += bytes_read as u64;
                    state.chunks_written += 1;
                    state.current_file_size = writer.current_size;
                    state.rotations = writer.rotations;
                }
            }
            Err(LocalSerialError::ReadTimeout) => {}
            Err(error) => {
                set_recording_error(&session, error.to_string()).await;
                break;
            }
        }

        if last_flush.elapsed() >= Duration::from_millis(session.options.flush_interval_ms) {
            if let Err(error) = writer.flush().await {
                set_recording_error(&session, error.to_string()).await;
                break;
            }
            last_flush = tokio::time::Instant::now();
        }
    }

    if let Err(error) = writer.flush().await {
        set_recording_error(&session, error.to_string()).await;
    }
    let mut state = session.state.lock().await;
    state.active = false;
    state.stopped_at = Some(Utc::now());
    state.current_file_size = writer.current_size;
    state.rotations = writer.rotations;
}

async fn set_recording_error(session: &RecordingSession, error: String) {
    session.state.lock().await.last_error = Some(error);
}

fn append_ring(ring: &mut VecDeque<u8>, bytes: &[u8], capacity: usize) {
    if bytes.len() >= capacity {
        ring.clear();
        ring.extend(bytes[bytes.len() - capacity..].iter().copied());
        return;
    }
    let overflow = ring
        .len()
        .saturating_add(bytes.len())
        .saturating_sub(capacity);
    if overflow > 0 {
        ring.drain(..overflow);
    }
    ring.extend(bytes.iter().copied());
}

#[derive(Debug)]
pub(crate) struct RotatingFileWriter {
    path: PathBuf,
    max_file_size_bytes: u64,
    max_files: usize,
    file: Option<File>,
    current_size: u64,
    rotations: u64,
}

impl RotatingFileWriter {
    pub(crate) async fn open(
        path: PathBuf,
        max_file_size_bytes: u64,
        max_files: usize,
    ) -> RecordingResult<Self> {
        let existing_size = match fs::metadata(&path).await {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error.into()),
        };

        if existing_size >= max_file_size_bytes {
            rotate_paths(&path, max_files).await?;
        }

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await?;
        let current_size = file.metadata().await?.len();
        Ok(Self {
            path,
            max_file_size_bytes,
            max_files,
            file: Some(file),
            current_size,
            rotations: 0,
        })
    }

    pub(crate) async fn write_all(&mut self, mut bytes: &[u8]) -> RecordingResult<()> {
        while !bytes.is_empty() {
            if self.current_size >= self.max_file_size_bytes {
                self.rotate().await?;
            }
            let available = (self.max_file_size_bytes - self.current_size) as usize;
            let take = available.min(bytes.len());
            self.file_mut()?.write_all(&bytes[..take]).await?;
            self.current_size += take as u64;
            bytes = &bytes[take..];
        }
        Ok(())
    }

    pub(crate) async fn flush(&mut self) -> RecordingResult<()> {
        self.file_mut()?.flush().await?;
        Ok(())
    }

    /// Keep each JSONL record intact across rotation boundaries.
    pub(crate) async fn write_record(&mut self, bytes: &[u8]) -> RecordingResult<()> {
        if self.current_size > 0
            && self.current_size + bytes.len() as u64 > self.max_file_size_bytes
        {
            self.rotate().await?;
        }
        self.file_mut()?.write_all(bytes).await?;
        self.current_size += bytes.len() as u64;
        Ok(())
    }

    async fn rotate(&mut self) -> RecordingResult<()> {
        if let Some(mut file) = self.file.take() {
            file.flush().await?;
            drop(file);
        }
        rotate_paths(&self.path, self.max_files).await?;
        self.file = Some(
            OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&self.path)
                .await?,
        );
        self.current_size = 0;
        self.rotations += 1;
        Ok(())
    }

    fn file_mut(&mut self) -> RecordingResult<&mut File> {
        self.file
            .as_mut()
            .ok_or_else(|| RecordingError::Task("recording file is not open".to_string()))
    }
}

async fn rotate_paths(path: &Path, max_files: usize) -> RecordingResult<()> {
    if max_files <= 1 {
        match fs::remove_file(path).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        return Ok(());
    }

    for index in (1..max_files).rev() {
        let destination = rotated_path(path, index);
        match fs::remove_file(&destination).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let source = if index == 1 {
            path.to_path_buf()
        } else {
            rotated_path(path, index - 1)
        };
        match fs::rename(&source, &destination).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn rotated_path(path: &Path, index: usize) -> PathBuf {
    let mut name: OsString = path.as_os_str().to_os_string();
    name.push(format!(".{index}"));
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_options_validate_bounds() {
        let valid = RecordingOptions {
            path: PathBuf::from("serial.log"),
            max_file_size_bytes: 1024,
            max_files: 3,
            read_chunk_size: 256,
            read_timeout_ms: 100,
            ring_buffer_bytes: 512,
            flush_interval_ms: 1000,
        };
        assert!(valid.validate().is_ok());

        let mut invalid = valid.clone();
        invalid.ring_buffer_bytes = 0;
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn ring_buffer_keeps_only_the_newest_bytes() {
        let mut ring = VecDeque::new();
        append_ring(&mut ring, b"abcd", 6);
        append_ring(&mut ring, b"efgh", 6);
        assert_eq!(ring.into_iter().collect::<Vec<_>>(), b"cdefgh");
    }

    #[tokio::test]
    async fn rotating_writer_limits_files_and_preserves_order() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("serial.log");
        let mut writer = RotatingFileWriter::open(path.clone(), 4, 3).await.unwrap();
        writer.write_all(b"abcdefghijkl").await.unwrap();
        writer.flush().await.unwrap();

        assert_eq!(fs::read(&path).await.unwrap(), b"ijkl");
        assert_eq!(fs::read(rotated_path(&path, 1)).await.unwrap(), b"efgh");
        assert_eq!(fs::read(rotated_path(&path, 2)).await.unwrap(), b"abcd");
        assert!(!rotated_path(&path, 3).exists());
    }
}
