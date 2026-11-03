use super::{model::*, tasks::Task};
use crate::recording::RotatingFileWriter;
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
    sync::{mpsc, oneshot, watch, Mutex, Notify, RwLock},
    task::JoinHandle,
};
use tokio_serial::{SerialPortBuilderExt, SerialStream};

pub struct Hub {
    pub data_dir: PathBuf,
    pub sessions: RwLock<HashMap<String, Arc<Session>>>,
    pub tasks: RwLock<HashMap<String, Arc<Task>>>,
    pub scripts: Mutex<Vec<super::scripts::SavedScript>>,
    pub detail_jobs: Mutex<Vec<JoinHandle<()>>>,
    opening: Mutex<()>,
    shutting_down: AtomicBool,
    pub shutdown_requested: Notify,
}

pub struct Session {
    pub journal: Mutex<Journal>,
    pub notify: Notify,
    pub gate: Arc<Mutex<()>>,
    pub queue_tail: Mutex<watch::Receiver<bool>>,
    commands: mpsc::Sender<DeviceCommand>,
    stop: watch::Sender<bool>,
    pub closed: AtomicBool,
    pub closing: AtomicBool,
    runner: Mutex<Option<JoinHandle<()>>>,
}

pub struct Journal {
    pub info: SessionInfo,
    pub ring: EventRing,
    raw: RotatingFileWriter,
    events: RotatingFileWriter,
}

enum DeviceCommand {
    Write {
        bytes: Vec<u8>,
        source: String,
        task: String,
        sensitive: bool,
        reply: oneshot::Sender<Result<(usize, u64)>>,
    },
    Control {
        request: ControlRequest,
        reply: oneshot::Sender<Result<()>>,
    },
}

enum Device {
    Serial(SerialStream),
    Demo(DuplexStream),
}
impl Device {
    async fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Serial(s) => s.read(buffer).await,
            Self::Demo(s) => s.read(buffer).await,
        }
    }
    async fn write(&mut self, data: &[u8]) -> std::io::Result<()> {
        match self {
            Self::Serial(s) => s.write_all(data).await,
            Self::Demo(s) => s.write_all(data).await,
        }
    }
    fn control(&mut self, request: &ControlRequest) -> Result<()> {
        use serialport::SerialPort;
        if let Self::Serial(s) = self {
            if let Some(level) = request.rts {
                s.write_request_to_send(level)?;
            }
            if let Some(level) = request.dtr {
                s.write_data_terminal_ready(level)?;
            }
        }
        Ok(())
    }
}

impl Hub {
    pub async fn new(data_dir: PathBuf) -> Result<Arc<Self>> {
        fs::create_dir_all(&data_dir).await?;
        let data_dir = fs::canonicalize(data_dir).await?;
        let scripts = super::scripts::load(&data_dir.join("scripts.json")).await?;
        let tasks = super::task_history::load_tasks(&data_dir.join("tasks")).await?;
        Ok(Arc::new(Self {
            data_dir,
            scripts: Mutex::new(scripts),
            detail_jobs: Mutex::new(Vec::new()),
            sessions: RwLock::new(HashMap::new()),
            tasks: RwLock::new(tasks),
            opening: Mutex::new(()),
            shutting_down: AtomicBool::new(false),
            shutdown_requested: Notify::new(),
        }))
    }

    pub async fn session(&self, id: &str) -> Result<Arc<Session>> {
        self.sessions
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow!("session not active in this daemon: {id}"))
    }

    pub async fn open(self: &Arc<Self>, mut request: OpenRequest) -> Result<SessionInfo> {
        let _opening = self.opening.lock().await;
        if self.shutting_down.load(Ordering::Acquire) {
            bail!("daemon is stopping");
        }
        if request.config.baud_rate == 0 || request.config.baud_rate > 4_000_000 {
            bail!("invalid baud rate");
        }
        if request.config.port.trim().is_empty() {
            bail!("port is required");
        }
        if request.demo {
            request.config.port = "DEMO".into();
        }
        let mut active = 0;
        for session in self.sessions.read().await.values() {
            let info = session.info().await;
            if info.state == "connected" {
                active += 1;
                if info.config.port.eq_ignore_ascii_case(&request.config.port) {
                    bail!("port already connected; attach to session {}", info.id);
                }
            }
        }
        if active >= 16 {
            bail!("maximum 16 active sessions");
        }
        let (device, demo_peer) = if request.demo {
            let (a, b) = tokio::io::duplex(64 * 1024);
            (Device::Demo(a), Some(b))
        } else {
            let c = &request.config;
            let stream = tokio_serial::new(&c.port, c.baud_rate)
                .data_bits(c.data_bits.into())
                .stop_bits(c.stop_bits.into())
                .parity(c.parity.into())
                .flow_control(c.flow_control.into())
                .open_native_async()
                .with_context(|| format!("cannot open {}", c.port))?;
            (Device::Serial(stream), None)
        };
        let id = uuid::Uuid::new_v4().to_string();
        let dir = self.data_dir.join(&id);
        fs::create_dir_all(&dir).await?;
        let info = SessionInfo {
            id: id.clone(),
            config: request.config,
            demo: request.demo,
            state: "connected".into(),
            started_at: now(),
            ended_at: None,
            rx_bytes: 0,
            tx_bytes: 0,
            last_rx_at: None,
            active_task: None,
            control_owner: None,
            last_error: None,
            directory: dir.to_string_lossy().into(),
            next_seq: 1,
            rts: None,
            dtr: None,
        };
        let journal = Journal {
            info,
            ring: EventRing::default(),
            raw: RotatingFileWriter::open(dir.join("rx.log"), LOG_BYTES, LOG_FILES).await?,
            events: RotatingFileWriter::open(dir.join("events.jsonl"), LOG_BYTES * 2, LOG_FILES)
                .await?,
        };
        let (commands, rx) = mpsc::channel(64);
        let (stop, stop_rx) = watch::channel(false);
        let (_, initial_tail) = watch::channel(true);
        let session = Arc::new(Session {
            queue_tail: Mutex::new(initial_tail),
            journal: Mutex::new(journal),
            notify: Notify::new(),
            gate: Arc::new(Mutex::new(())),
            commands,
            stop,
            closed: AtomicBool::new(false),
            closing: AtomicBool::new(false),
            runner: Mutex::new(None),
        });
        session
            .system("daemon", "Session opened; capture started", None)
            .await?;
        session.persist().await?;
        self.sessions.write().await.insert(id, session.clone());
        if let Some(peer) = demo_peer {
            tokio::spawn(demo_device(peer));
        }
        let clone = session.clone();
        *session.runner.lock().await = Some(tokio::spawn(async move {
            device_loop(clone, device, rx, stop_rx).await
        }));
        Ok(session.info().await)
    }

    pub async fn list(&self) -> Result<Vec<SessionInfo>> {
        let mut infos = HashMap::new();
        let mut dirs = fs::read_dir(&self.data_dir).await?;
        while let Some(entry) = dirs.next_entry().await? {
            if !entry.file_type().await?.is_dir() {
                continue;
            }
            if let Ok(bytes) = fs::read(entry.path().join("session.json")).await {
                if let Ok(mut info) = serde_json::from_slice::<SessionInfo>(&bytes) {
                    if info.state == "connected" {
                        info.state = "interrupted".into();
                        info.active_task = None;
                    }
                    infos.insert(info.id.clone(), info);
                }
            }
        }
        for (id, session) in self.sessions.read().await.iter() {
            infos.insert(id.clone(), session.info().await);
        }
        let mut result: Vec<_> = infos.into_values().collect();
        result.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        Ok(result)
    }

    pub async fn events(&self, id: &str, query: &EventQuery, history: bool) -> Result<EventPage> {
        if !history {
            if let Ok(session) = self.session(id).await {
                return Ok(session.page(query).await);
            }
        }
        let path = self.session_dir(id)?;
        let query = query.clone();
        tokio::task::spawn_blocking(move || read_history(&path, &query)).await?
    }

    pub fn session_dir(&self, id: &str) -> Result<PathBuf> {
        let parsed = uuid::Uuid::parse_str(id).context("invalid session id")?;
        Ok(self.data_dir.join(parsed.to_string()))
    }

    pub async fn close(&self, id: &str) -> Result<SessionInfo> {
        let session = self.session(id).await?;
        session.closing.store(true, Ordering::Release);
        self.cancel_session(id).await;
        let mut tail = session.queue_tail.lock().await.clone();
        let _ = tail.wait_for(|done| *done).await;
        session.stop.send_replace(true);
        if let Some(task) = session.runner.lock().await.take() {
            task.await?;
        }
        let info = session.info().await;
        self.sessions.write().await.remove(id);
        Ok(info)
    }

    pub async fn shutdown(&self) {
        self.shutting_down.store(true, Ordering::Release);
        let opening = self.opening.lock().await;
        let ids: Vec<_> = self.sessions.read().await.keys().cloned().collect();
        drop(opening);
        for id in ids {
            let _ = self.close(&id).await;
        }
        let jobs = std::mem::take(&mut *self.detail_jobs.lock().await);
        for job in jobs {
            let _ = job.await;
        }
    }
}

impl Session {
    pub async fn info(&self) -> SessionInfo {
        self.journal.lock().await.info.clone()
    }
    pub async fn page(&self, query: &EventQuery) -> EventPage {
        self.journal.lock().await.ring.page(query)
    }
    pub async fn cursor(&self) -> u64 {
        self.journal.lock().await.info.next_seq.saturating_sub(1)
    }

    pub async fn record(
        &self,
        direction: &str,
        source: &str,
        bytes: &[u8],
        task: Option<String>,
        sensitive: bool,
    ) -> Result<()> {
        let mut j = self.journal.lock().await;
        let time = now();
        let offset = j.info.rx_bytes;
        if direction == "rx" {
            j.raw.write_all(bytes).await?;
            j.info.rx_bytes += bytes.len() as u64;
            j.info.last_rx_at = Some(time.clone());
        }
        if direction == "tx" {
            j.info.tx_bytes += bytes.len() as u64;
        }
        let event = Event {
            seq: j.info.next_seq,
            time,
            direction: direction.into(),
            source: source.into(),
            text: if sensitive {
                "[hidden input]".into()
            } else {
                String::from_utf8_lossy(bytes).into_owned()
            },
            base64: if sensitive {
                String::new()
            } else {
                STANDARD.encode(bytes)
            },
            bytes: bytes.len(),
            rx_offset: offset,
            task_id: task,
        };
        let mut line = serde_json::to_vec(&event)?;
        line.push(b'\n');
        j.events.write_record(&line).await?;
        let event = j.ring.push(event);
        j.info.next_seq = event.seq + 1;
        drop(j);
        self.notify.notify_waiters();
        Ok(())
    }

    pub async fn system(&self, source: &str, text: &str, task: Option<String>) -> Result<()> {
        self.record("system", source, text.as_bytes(), task, false)
            .await
    }
    pub async fn persist(&self) -> Result<()> {
        let mut j = self.journal.lock().await;
        j.raw.flush().await?;
        j.events.flush().await?;
        fs::write(
            Path::new(&j.info.directory).join("session.json"),
            serde_json::to_vec_pretty(&j.info)?,
        )
        .await?;
        Ok(())
    }
    pub async fn send(
        &self,
        bytes: Vec<u8>,
        source: String,
        task: String,
        sensitive: bool,
    ) -> Result<(usize, u64)> {
        if self.closed.load(Ordering::Acquire) || self.closing.load(Ordering::Acquire) {
            bail!("session is closed");
        }
        let (reply, result) = oneshot::channel();
        self.commands
            .send(DeviceCommand::Write {
                bytes,
                source,
                task,
                sensitive,
                reply,
            })
            .await
            .map_err(|_| anyhow!("session is closed"))?;
        result.await.context("device disconnected")?
    }
    pub async fn control(&self, request: ControlRequest) -> Result<()> {
        if self.closed.load(Ordering::Acquire) || self.closing.load(Ordering::Acquire) {
            bail!("session is closed");
        }
        if request.rts.is_none() && request.dtr.is_none() {
            bail!("provide RTS or DTR");
        }
        let _gate = self.gate.lock().await;
        if let Some(owner) = &self.journal.lock().await.info.control_owner {
            if owner != &request.source {
                bail!("control held by {owner}; wait for the user to release it");
            }
        }
        let (reply, result) = oneshot::channel();
        self.commands
            .send(DeviceCommand::Control { request, reply })
            .await
            .map_err(|_| anyhow!("session is closed"))?;
        result.await.context("device disconnected")?
    }
}

async fn device_loop(
    session: Arc<Session>,
    mut device: Device,
    mut commands: mpsc::Receiver<DeviceCommand>,
    mut stop: watch::Receiver<bool>,
) {
    let mut buffer = vec![0; 8192];
    let mut flush = tokio::time::interval(Duration::from_secs(1));
    let failure: Option<String> = loop {
        tokio::select! {
            biased;
            _ = stop.changed() => break None,
            command = commands.recv() => match command {
                Some(DeviceCommand::Write { bytes, source, task, sensitive, reply }) => {
                    if reply.is_closed() { continue; }
                    let result = async {
                        tokio::time::timeout(Duration::from_secs(3), device.write(&bytes)).await.context("write timed out; delivery may be partial")??;
                        session.record("tx", &source, &bytes, Some(task), sensitive).await?;
                        Ok((bytes.len(), session.cursor().await))
                    }.await;
                    let error = result.as_ref().err().map(|e: &anyhow::Error| e.to_string());
                    let _ = reply.send(result);
                    if error.is_some() { break error; }
                }
                Some(DeviceCommand::Control { request, reply }) => {
                    let result = device.control(&request);
                    if result.is_ok() {
                        { let mut j = session.journal.lock().await; if request.rts.is_some() { j.info.rts = request.rts; }
                        if request.dtr.is_some() { j.info.dtr = request.dtr; } }
                        let text = format!("Control lines: RTS={:?}, DTR={:?}", request.rts, request.dtr);
                        if let Err(e) = session.system(&request.source, &text, None).await { let _ = reply.send(Err(e)); break Some("journal write failed".into()); }
                    }
                    let _ = reply.send(result);
                }
                None => break None,
            },
            result = device.read(&mut buffer) => match result {
                Ok(0) => break Some("device disconnected (EOF)".into()),
                Ok(n) => if let Err(e) = session.record("rx", "device", &buffer[..n], None, false).await { break Some(e.to_string()); },
                Err(e) => break Some(e.to_string()),
            },
            _ = flush.tick() => if let Err(e) = session.persist().await { break Some(e.to_string()); },
        }
    };
    session.closed.store(true, Ordering::Release);
    drop(device);
    {
        let mut j = session.journal.lock().await;
        j.info.state = if failure.is_some() { "error" } else { "closed" }.into();
        j.info.last_error = failure.clone();
        j.info.ended_at = Some(now());
        j.info.control_owner = None;
    }
    let message = failure
        .map(|s| format!("Session stopped: {s}"))
        .unwrap_or_else(|| "Session closed; capture flushed".into());
    let _ = session.system("daemon", &message, None).await;
    let _ = session.persist().await;
    session.notify.notify_waiters();
}

fn read_history(dir: &Path, query: &EventQuery) -> Result<EventPage> {
    use std::io::{BufRead, BufReader};
    if !dir.join("session.json").is_file() {
        bail!("session not found");
    }
    let mut events = Vec::new();
    let limit = query.limit.unwrap_or(200).clamp(1, 1000);
    let filter = query.contains.as_deref().unwrap_or("").to_lowercase();
    let mut first = 0;
    let mut cursor = query.after;
    let mut more = false;
    'files: for index in (0..LOG_FILES).rev() {
        let name = if index == 0 {
            "events.jsonl".into()
        } else {
            format!("events.jsonl.{index}")
        };
        let file = match std::fs::File::open(dir.join(name)) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        for line in BufReader::new(file).lines() {
            let line = line?;
            let event: Event = match serde_json::from_str(&line) {
                Ok(e) => e,
                Err(_) => continue,
            };
            if first == 0 {
                first = event.seq;
            }
            if event.seq <= query.after {
                continue;
            }
            if events.len() >= limit {
                more = true;
                break 'files;
            }
            cursor = event.seq;
            if filter.is_empty() || event.text.to_lowercase().contains(&filter) {
                events.push(event);
            }
        }
    }
    Ok(EventPage {
        events,
        next_cursor: cursor,
        first_available: first,
        gap: first > query.after.saturating_add(1),
        has_more: more,
    })
}

async fn demo_device(mut stream: DuplexStream) {
    let mut interval = tokio::time::interval(Duration::from_millis(500));
    let mut buffer = [0; 8192];
    let mut count = 0u64;
    loop {
        tokio::select! {
            _ = interval.tick() => {
                count += 1;
                let level = if count % 12 == 0 { "WARN" } else { "INFO" };
                let line = format!("[{}] [{level}] DEMO Detect({count}) speed=85.00 心跳正常\r\n", now());
                if stream.write_all(line.as_bytes()).await.is_err() { break; }
            }
            result = stream.read(&mut buffer) => match result {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let reply = format!("\r\nDEMO received: {}\r\nOK\r\ndemo> ", String::from_utf8_lossy(&buffer[..n]).trim());
                    if stream.write_all(reply.as_bytes()).await.is_err() { break; }
                }
            }
        }
    }
}
