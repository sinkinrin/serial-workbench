use super::{
    model::*,
    session::{Hub, Session},
};
use crate::automation::{
    plan_target, AutomationError, AutomationResult, Encoding, ExpectOperation, MacroExecutor,
    MacroPack, MacroPlan, MacroTarget, MacroTransport, PlanStep,
};
use anyhow::{anyhow, bail, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use tokio::sync::{watch, Mutex};

pub struct Task {
    pub info: Mutex<TaskInfo>,
    pub cancel: watch::Sender<bool>,
    pub steps: Vec<TaskStep>,
    pub script: Option<ScriptReference>,
    pub transcript: Mutex<TaskTranscript>,
    pub reconstructed: bool,
    pub path: PathBuf,
    pub persist_lock: Mutex<()>,
}
struct TaskSpec {
    source: String,
    label: String,
    plan: MacroPlan,
    sensitive: bool,
    success: &'static str,
    script: Option<ScriptReference>,
}
fn visible_command(bytes: &[u8]) -> String {
    let mut text = String::new();
    for ch in String::from_utf8_lossy(bytes).chars() {
        match ch {
            '\r' => text.push_str("⟨CR⟩"),
            '\n' => text.push_str("⟨LF⟩"),
            '\t' => text.push_str("⟨TAB⟩"),
            c if c.is_control() => text.push_str(&format!("⟨0x{:02X}⟩", c as u32)),
            c => text.push(c),
        }
    }
    text
}
fn plan_steps(plan: &MacroPlan, sensitive: bool) -> Vec<TaskStep> {
    plan.steps
        .iter()
        .map(|step| match step {
            PlanStep::Send { data, encoding, .. } => TaskStep {
                kind: "send".into(),
                data: Some(if sensitive {
                    "[hidden input]".into()
                } else {
                    data.clone()
                }),
                encoding: Some(encoding.as_str().into()),
                ms: None,
                timeout_ms: None,
            },
            PlanStep::Delay { ms, .. } => TaskStep {
                kind: "delay".into(),
                data: None,
                encoding: None,
                ms: Some(*ms),
                timeout_ms: None,
            },
            PlanStep::Expect {
                data,
                encoding,
                timeout_ms,
                op,
                ..
            } => TaskStep {
                kind: format!(
                    "expect_{}",
                    if matches!(op, ExpectOperation::Contains) {
                        "contains"
                    } else {
                        "equals"
                    }
                ),
                data: Some(if sensitive {
                    "[hidden expectation]".into()
                } else {
                    data.clone()
                }),
                encoding: Some(encoding.as_str().into()),
                ms: None,
                timeout_ms: Some(*timeout_ms),
            },
        })
        .collect()
}

impl Hub {
    pub async fn send(self: &Arc<Self>, id: &str, request: SendRequest) -> Result<TaskInfo> {
        if request.timeout_ms == 0 || request.timeout_ms > 300_000 {
            bail!("timeout_ms must be 1..300000");
        }
        let encoding = match request.encoding.as_str() {
            "utf8" => Encoding::Utf8,
            "hex" => Encoding::Hex,
            "base64" => Encoding::Base64,
            _ => bail!("encoding must be utf8, hex or base64"),
        };
        let bytes = encoding.decode(&request.data, "data")?;
        if bytes.is_empty() || bytes.len() > 65536 {
            bail!("send requires 1..65536 bytes");
        }
        if request
            .wait_for
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.len() > 65536)
        {
            bail!("wait_for requires 1..65536 bytes");
        }
        let label = if request.sensitive {
            "Hidden input".into()
        } else {
            visible_command(&bytes).chars().take(96).collect()
        };
        let mut steps = vec![PlanStep::Send {
            source_macro: "send".into(),
            data: request.data,
            encoding,
        }];
        let success = if let Some(expected) = request.wait_for {
            steps.push(PlanStep::Expect {
                source_macro: "send".into(),
                op: ExpectOperation::Contains,
                data: expected,
                encoding: Encoding::Utf8,
                timeout_ms: request.timeout_ms,
                idle_ms: 100,
                max_bytes: 1024 * 1024,
                trim: false,
            });
            "succeeded"
        } else {
            "sent"
        };
        self.start_task(
            id,
            TaskSpec {
                source: request.source,
                label,
                plan: MacroPlan {
                    pack_name: "workbench".into(),
                    target_kind: "command".into(),
                    target_name: "send".into(),
                    steps,
                },
                sensitive: request.sensitive,
                success,
                script: None,
            },
        )
        .await
    }

    pub async fn run_macro(self: &Arc<Self>, id: &str, request: MacroRequest) -> Result<TaskInfo> {
        self.run_macro_from_script(id, request, None).await
    }

    pub async fn run_macro_from_script(
        self: &Arc<Self>,
        id: &str,
        request: MacroRequest,
        script: Option<ScriptReference>,
    ) -> Result<TaskInfo> {
        let pack: MacroPack = serde_json::from_value(request.pack)?;
        let target = if request.assembly {
            MacroTarget::assembly_named(&request.name)
        } else {
            MacroTarget::macro_named(&request.name)
        };
        let plan = plan_target(&pack, target)?;
        let success = if plan
            .steps
            .iter()
            .any(|step| matches!(step, PlanStep::Expect { .. }))
        {
            "succeeded"
        } else {
            "completed"
        };
        self.start_task(
            id,
            TaskSpec {
                source: request.source,
                label: format!("脚本：{}", request.name),
                plan,
                sensitive: false,
                success,
                script,
            },
        )
        .await
    }

    async fn start_task(self: &Arc<Self>, id: &str, spec: TaskSpec) -> Result<TaskInfo> {
        let TaskSpec {
            source,
            label,
            plan,
            sensitive,
            success,
            script,
        } = spec;
        let commands: Vec<_> = plan
            .steps
            .iter()
            .filter_map(|step| {
                if let PlanStep::Send { data, encoding, .. } = step {
                    Some(if sensitive {
                        "[hidden input]".into()
                    } else {
                        encoding
                            .decode(data, "data")
                            .map(|bytes| visible_command(&bytes))
                            .unwrap_or_else(|_| data.clone())
                    })
                } else {
                    None
                }
            })
            .collect();
        if source.is_empty() || source.len() > 64 {
            bail!("source requires 1..64 characters");
        }
        let session = self.session(id).await?;
        if session.closed.load(Ordering::Acquire) || session.closing.load(Ordering::Acquire) {
            bail!("session is closed");
        }
        let info = TaskInfo {
            id: uuid::Uuid::new_v4().to_string(),
            port: session.info().await.config.port,
            session_id: id.into(),
            source,
            label,
            status: "queued".into(),
            created_at: now(),
            finished_at: None,
            start_cursor: 0,
            end_cursor: 0,
            message: None,
            command_count: commands.len(),
            commands_preview: commands
                .iter()
                .take(3)
                .map(|s| s.chars().take(180).collect())
                .collect(),
            sensitive,
        };
        let task = Arc::new(Task::from_details(
            TaskDetails {
                info: info.clone(),
                steps: plan_steps(&plan, sensitive),
                script,
                transcript: TaskTranscript::default(),
                reconstructed: false,
            },
            super::task_history::task_path(&self.data_dir, &info.id)?,
        ));
        let cancel_rx = task.cancel.subscribe();
        let mut tail = session.queue_tail.lock().await;
        if let Some(owner) = &session.journal.lock().await.info.control_owner {
            if owner != &info.source {
                bail!("control held by {owner}; wait for the user to release it");
            }
        }
        {
            let mut tasks = self.tasks.write().await;
            // Bound resident task metadata; complete task outcomes also live in the event journal.
            if tasks.len() >= 1000 {
                let mut finished = Vec::new();
                for (key, item) in tasks.iter() {
                    if item.info.lock().await.finished_at.is_some() {
                        finished.push(key.clone());
                    }
                }
                for key in finished {
                    tasks.remove(&key);
                }
            }
            if tasks.len() >= 1000 {
                bail!("too many pending tasks");
            }
            task.persist().await?;
            tasks.insert(info.id.clone(), task.clone());
        }
        if let Err(error) = session
            .system(
                &info.source,
                &format!("Task queued: {}", info.label),
                Some(info.id.clone()),
            )
            .await
        {
            {
                let mut failed = task.info.lock().await;
                failed.status = "failed".into();
                failed.finished_at = Some(now());
                failed.message = Some(error.to_string());
            }
            let _ = task.persist().await;
            return Err(error);
        }
        let mut previous = tail.clone();
        let (finished, next) = watch::channel(false);
        *tail = next;
        drop(tail);
        let hub = self.clone();
        tokio::spawn(async move {
            let _ = previous.wait_for(|done| *done).await;
            execute(session, task.clone(), cancel_rx, plan, sensitive, success).await;
            if let Err(error) = task.persist().await {
                tracing::error!("Task persistence failed: {error}");
            }
            let task_id = task.info.lock().await.id.clone();
            let collector_hub = hub.clone();
            let job = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(1100)).await;
                if let Err(error) = collector_hub.task_details(&task_id).await {
                    tracing::warn!("Task receive window unavailable: {error}");
                }
            });
            {
                let mut jobs = hub.detail_jobs.lock().await;
                jobs.retain(|job| !job.is_finished());
                jobs.push(job);
            }
            finished.send_replace(true);
        });
        Ok(info)
    }

    pub async fn task(&self, id: &str) -> Result<TaskInfo> {
        if let Some(task) = self.tasks.read().await.get(id).cloned() {
            return Ok(task.info.lock().await.clone());
        }
        Ok(self.task_details(id).await?.info)
    }
    pub async fn task_list(&self) -> Vec<TaskInfo> {
        self.task_list_for(None).await
    }
    pub async fn task_list_for(&self, session_id: Option<&str>) -> Vec<TaskInfo> {
        let mut result = Vec::new();
        for item in self.tasks.read().await.values() {
            let info = item.info.lock().await.clone();
            if session_id.is_none_or(|id| info.session_id == id) {
                result.push(info);
            }
        }
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        result.truncate(200);
        result
    }
    pub async fn cancel_task(&self, id: &str) -> Result<TaskInfo> {
        let item = self
            .tasks
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow!("task not found"))?;
        item.cancel.send_replace(true);
        let info = item.info.lock().await.clone();
        Ok(info)
    }
    pub async fn cancel_session(&self, session_id: &str) {
        for task in self.tasks.read().await.values() {
            if task.info.lock().await.session_id == session_id {
                task.cancel.send_replace(true);
            }
        }
    }
    pub async fn release_control(&self, id: &str) -> Result<()> {
        let session = self.session(id).await?;
        let _queue = session.queue_tail.lock().await;
        session.journal.lock().await.info.control_owner = None;
        session
            .system(
                "human",
                "User released control; shared queue available",
                None,
            )
            .await
    }

    pub async fn takeover(&self, id: &str) -> Result<()> {
        let session = self.session(id).await?;
        let queue = session.queue_tail.lock().await;
        session.journal.lock().await.info.control_owner = Some("human".into());
        self.cancel_session(id).await;
        drop(queue);
        let _gate = session.gate.lock().await;
        session
            .system(
                "human",
                "User holds control; other sources blocked until released (no bytes sent to device)",
                None,
            )
            .await
    }
}

async fn execute(
    session: Arc<Session>,
    task: Arc<Task>,
    mut cancel: watch::Receiver<bool>,
    plan: MacroPlan,
    sensitive: bool,
    success: &str,
) {
    let outcome = async {
        if *cancel.borrow() { bail!("cancelled"); }
        let _guard = tokio::select! {
            biased;
            _ = cancel.changed() => return Err(anyhow!("cancelled")),
            guard = session.gate.lock() => guard,
        };
        if *cancel.borrow() { bail!("cancelled"); }
        if session.closed.load(Ordering::Acquire) || session.closing.load(Ordering::Acquire) { bail!("session closed"); }
        let cursor = session.cursor().await;
        let (id, source) = {
            let mut info = task.info.lock().await;
            info.status = "running".into(); info.start_cursor = cursor;
            (info.id.clone(), info.source.clone())
        };
        task.persist().await?;
        session.journal.lock().await.info.active_task = Some(id.clone());
        session.system(&source, "Task started", Some(id.clone())).await?;
        let transport = SharedTransport { session: session.clone(), cursor, pending: VecDeque::new(), source, task: id, sensitive };
        let executor = MacroExecutor::real();
        tokio::select! {
            biased;
            _ = cancel.changed() => Err(anyhow!("cancelled; already transmitted bytes cannot be recalled")),
            result = executor.run(plan, transport) => result.map(|_| ()).map_err(anyhow::Error::from),
        }
    }.await;
    let cursor = session.cursor().await;
    let (id, source, status, message) = {
        let mut info = task.info.lock().await;
        info.status = match &outcome {
            Ok(_) => success.into(),
            Err(_) if *cancel.borrow() => "cancelled".into(),
            Err(_) => "failed".into(),
        };
        info.message = outcome.err().map(|e| e.to_string());
        info.finished_at = Some(now());
        info.end_cursor = cursor;
        (
            info.id.clone(),
            info.source.clone(),
            info.status.clone(),
            info.message.clone(),
        )
    };
    {
        let mut journal = session.journal.lock().await;
        if journal.info.active_task.as_deref() == Some(&id) {
            journal.info.active_task = None;
        }
    }
    let _ = session
        .system(
            &source,
            &format!(
                "Task {status}{}",
                message.map(|s| format!(": {s}")).unwrap_or_default()
            ),
            Some(id),
        )
        .await;
}

struct SharedTransport {
    session: Arc<Session>,
    cursor: u64,
    pending: VecDeque<u8>,
    source: String,
    task: String,
    sensitive: bool,
}

#[async_trait::async_trait]
impl MacroTransport for SharedTransport {
    async fn write(&mut self, data: &[u8]) -> AutomationResult<usize> {
        // Each send starts a new response window; never satisfy it from earlier unsolicited data.
        self.pending.clear();
        let (written, cursor) = self
            .session
            .send(
                data.to_vec(),
                self.source.clone(),
                self.task.clone(),
                self.sensitive,
            )
            .await
            .map_err(|e| AutomationError::Transport(e.to_string()))?;
        self.cursor = cursor;
        Ok(written)
    }
    async fn read(&mut self, max_bytes: usize, timeout_ms: u64) -> AutomationResult<Vec<u8>> {
        let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            if !self.pending.is_empty() {
                let n = max_bytes.min(self.pending.len());
                return Ok(self.pending.drain(..n).collect());
            }
            let page = self
                .session
                .page(&EventQuery {
                    after: self.cursor,
                    limit: Some(200),
                    contains: None,
                })
                .await;
            if page.gap {
                return Err(AutomationError::Transport(
                    "receive history overflowed during task; outcome cannot be verified".into(),
                ));
            }
            self.cursor = page.next_cursor;
            for event in page.events {
                if event.direction == "rx" {
                    self.pending.extend(
                        STANDARD
                            .decode(&event.base64)
                            .map_err(|e| AutomationError::Transport(e.to_string()))?,
                    );
                }
            }
            if !self.pending.is_empty() || page.has_more {
                continue;
            }
            if self.session.closed.load(Ordering::Acquire) {
                return Err(AutomationError::Transport("device disconnected".into()));
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(Vec::new());
            }
            // Bounded wakeup also handles a notification arriving between snapshot and wait.
            let wait = Duration::from_millis(20)
                .min(deadline.saturating_duration_since(tokio::time::Instant::now()));
            let _ = tokio::time::timeout(wait, self.session.notify.notified()).await;
        }
    }
}
