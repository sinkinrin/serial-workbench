use crate::serial::ConnectionConfig;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

pub const EVENT_MEMORY_BYTES: usize = 4 * 1024 * 1024;
pub const EVENT_LIMIT: usize = 4096;
pub const LOG_BYTES: u64 = 10 * 1024 * 1024;
pub const LOG_FILES: usize = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenRequest {
    #[serde(flatten)]
    pub config: ConnectionConfig,
    #[serde(default)]
    pub demo: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub config: ConnectionConfig,
    pub demo: bool,
    pub state: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub last_rx_at: Option<String>,
    pub active_task: Option<String>,
    #[serde(default)]
    pub control_owner: Option<String>,
    pub last_error: Option<String>,
    pub directory: String,
    pub next_seq: u64,
    pub rts: Option<bool>,
    pub dtr: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    pub time: String,
    pub direction: String,
    pub source: String,
    pub text: String,
    pub base64: String,
    pub bytes: usize,
    pub rx_offset: u64,
    pub task_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EventQuery {
    #[serde(default)]
    pub after: u64,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub contains: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventPage {
    pub events: Vec<Event>,
    pub next_cursor: u64,
    pub first_available: u64,
    pub gap: bool,
    pub has_more: bool,
}

pub struct EventRing {
    events: VecDeque<Event>,
    bytes: usize,
    next_seq: u64,
}

impl Default for EventRing {
    fn default() -> Self {
        Self {
            events: VecDeque::new(),
            bytes: 0,
            next_seq: 1,
        }
    }
}

impl EventRing {
    pub fn push(&mut self, mut event: Event) -> Event {
        event.seq = self.next_seq;
        self.next_seq += 1;
        self.bytes += event.text.len() + event.base64.len() + 256;
        self.events.push_back(event.clone());
        while self.bytes > EVENT_MEMORY_BYTES || self.events.len() > EVENT_LIMIT {
            if let Some(old) = self.events.pop_front() {
                self.bytes -= old.text.len() + old.base64.len() + 256;
            }
        }
        event
    }

    pub fn page(&self, query: &EventQuery) -> EventPage {
        page_events(
            self.events.iter().cloned(),
            query,
            self.next_seq.saturating_sub(1),
        )
    }
}

pub fn page_events(
    events: impl Iterator<Item = Event>,
    query: &EventQuery,
    latest: u64,
) -> EventPage {
    let limit = query.limit.unwrap_or(200).clamp(1, 1000);
    let mut page = EventPage {
        events: Vec::new(),
        next_cursor: query.after,
        first_available: 0,
        gap: false,
        has_more: false,
    };
    let filter = query.contains.as_deref().unwrap_or("").to_lowercase();
    for event in events {
        if page.first_available == 0 {
            page.first_available = event.seq;
        }
        if event.seq <= query.after {
            continue;
        }
        if page.events.len() >= limit {
            page.has_more = true;
            break;
        }
        page.next_cursor = event.seq;
        if filter.is_empty() || event.text.to_lowercase().contains(&filter) {
            page.events.push(event);
        }
    }
    if !page.has_more {
        page.next_cursor = page.next_cursor.max(latest);
    }
    page.gap = page.first_available > query.after.saturating_add(1);
    page
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendRequest {
    pub data: String,
    #[serde(default = "utf8")]
    pub encoding: String,
    #[serde(default = "cli")]
    pub source: String,
    #[serde(default)]
    pub wait_for: Option<String>,
    #[serde(default = "timeout")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub sensitive: bool,
}
fn utf8() -> String {
    "utf8".into()
}
fn cli() -> String {
    "cli".into()
}
fn timeout() -> u64 {
    5000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacroRequest {
    pub pack: serde_json::Value,
    pub name: String,
    #[serde(default)]
    pub assembly: bool,
    #[serde(default = "cli")]
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlRequest {
    pub rts: Option<bool>,
    pub dtr: Option<bool>,
    #[serde(default = "cli")]
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskInfo {
    pub id: String,
    #[serde(default)]
    pub port: String,
    pub session_id: String,
    pub source: String,
    pub label: String,
    pub status: String,
    pub created_at: String,
    pub finished_at: Option<String>,
    pub start_cursor: u64,
    pub end_cursor: u64,
    pub message: Option<String>,
    #[serde(default)]
    pub commands_preview: Vec<String>,
    #[serde(default)]
    pub command_count: usize,
    #[serde(default)]
    pub sensitive: bool,
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptReference {
    pub id: String,
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStep {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskTranscript {
    pub events: Vec<Event>,
    pub bytes_returned: usize,
    pub gap: bool,
    pub truncated: bool,
    pub finalized: bool,
    pub hidden: bool,
    pub next_cursor: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDetails {
    #[serde(flatten)]
    pub info: TaskInfo,
    pub steps: Vec<TaskStep>,
    #[serde(default)]
    pub script: Option<ScriptReference>,
    #[serde(default)]
    pub transcript: TaskTranscript,
    #[serde(default)]
    pub reconstructed: bool,
}
