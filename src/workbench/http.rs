use super::{model::*, scripts::*, Hub};
use anyhow::Result;
use axum::{
    body::Body,
    extract::{ws::Message, Path, Query, State, WebSocketUpgrade},
    http::{header, HeaderMap, Request, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures::SinkExt;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::io::AsyncReadExt;

#[derive(Clone)]
struct App {
    hub: Arc<Hub>,
    address: SocketAddr,
}
struct ApiError(anyhow::Error);
impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        Self(e.into())
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error":self.0.to_string()})),
        )
            .into_response()
    }
}
type Api<T> = std::result::Result<Json<T>, ApiError>;

pub async fn serve(hub: Arc<Hub>, port: u16) -> Result<()> {
    let listener =
        tokio::net::TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port))
            .await?;
    let address = listener.local_addr()?;
    eprintln!(
        "Serial Workbench: http://{address}\nSession files: {}",
        hub.data_dir.display()
    );
    let app = router(hub.clone(), address);
    let stopping = hub.clone();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {},
                _ = stopping.shutdown_requested.notified() => {},
            }
            stopping.shutdown().await;
        })
        .await?;
    hub.shutdown().await;
    Ok(())
}

pub fn router(hub: Arc<Hub>, address: SocketAddr) -> Router {
    let state = App { hub, address };
    Router::new()
        .route(
            "/",
            get(|| async { Html(include_str!("../../web/dist/index.html")) }),
        )
        .route(
            "/app.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../../web/dist/app.js"),
                )
            }),
        )
        .route(
            "/style.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    include_str!("../../web/dist/style.css"),
                )
            }),
        )
        .route("/api/status", get(status))
        .route("/api/shutdown", post(shutdown))
        .route("/api/ports", get(ports))
        .route("/api/sessions", get(sessions).post(open))
        .route("/api/sessions/{id}/events", get(events))
        .route("/api/sessions/{id}/history", get(history))
        .route("/api/sessions/{id}/stream", get(stream))
        .route("/api/sessions/{id}/send", post(send))
        .route("/api/sessions/{id}/macro", post(run_macro))
        .route("/api/sessions/{id}/control", post(control))
        .route("/api/sessions/{id}/close", post(close))
        .route("/api/sessions/{id}/takeover", post(takeover))
        .route("/api/sessions/{id}/release", post(release_control))
        .route("/api/sessions/{id}/export", get(export))
        .route("/api/tasks", get(tasks))
        .route("/api/tasks/{id}", get(task))
        .route("/api/tasks/{id}/details", get(task_details))
        .route("/api/scripts", get(scripts).post(save_script))
        .route("/api/scripts/{id}", get(script))
        .route("/api/scripts/{id}/delete", post(delete_script))
        .route("/api/scripts/{id}/run", post(run_script))
        .route("/api/tasks/{id}/cancel", post(cancel))
        .layer(axum::extract::DefaultBodyLimit::max(2 * 1024 * 1024))
        .layer(middleware::from_fn_with_state(state.clone(), local_only))
        .with_state(state)
}

async fn local_only(State(app): State<App>, request: Request<Body>, next: Next) -> Response {
    let allowed = [
        app.address.to_string(),
        format!("localhost:{}", app.address.port()),
    ];
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !allowed.iter().any(|s| s == host) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if let Some(origin) = request.headers().get(header::ORIGIN) {
        let origin = origin.to_str().unwrap_or("");
        if !allowed.iter().any(|s| origin == format!("http://{s}")) {
            return StatusCode::FORBIDDEN.into_response();
        }
    }
    if request.method() != axum::http::Method::GET
        && request
            .headers()
            .get("x-serial-workbench")
            .and_then(|v| v.to_str().ok())
            != Some("1")
    {
        return (StatusCode::FORBIDDEN, "missing X-Serial-Workbench: 1").into_response();
    }
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
        .headers_mut()
        .insert("x-content-type-options", "nosniff".parse().unwrap());
    response
}

async fn status(State(app): State<App>) -> Json<serde_json::Value> {
    Json(
        serde_json::json!({"name":"Serial Workbench","version":env!("CARGO_PKG_VERSION"),"data_dir":app.hub.data_dir,"address":app.address.to_string()}),
    )
}
async fn ports() -> Api<Vec<crate::serial::PortInfo>> {
    Ok(Json(crate::serial::PortInfo::list_ports()?))
}
async fn sessions(State(app): State<App>) -> Api<Vec<SessionInfo>> {
    Ok(Json(app.hub.list().await?))
}
async fn open(State(app): State<App>, Json(request): Json<OpenRequest>) -> Api<SessionInfo> {
    Ok(Json(app.hub.open(request).await?))
}
async fn close(State(app): State<App>, Path(id): Path<String>) -> Api<SessionInfo> {
    Ok(Json(app.hub.close(&id).await?))
}
async fn events(
    State(app): State<App>,
    Path(id): Path<String>,
    Query(q): Query<EventQuery>,
) -> Api<EventPage> {
    Ok(Json(app.hub.events(&id, &q, false).await?))
}
async fn history(
    State(app): State<App>,
    Path(id): Path<String>,
    Query(q): Query<EventQuery>,
) -> Api<EventPage> {
    Ok(Json(app.hub.events(&id, &q, true).await?))
}
async fn send(
    State(app): State<App>,
    Path(id): Path<String>,
    Json(q): Json<SendRequest>,
) -> Api<TaskInfo> {
    Ok(Json(app.hub.send(&id, q).await?))
}
async fn run_macro(
    State(app): State<App>,
    Path(id): Path<String>,
    Json(q): Json<MacroRequest>,
) -> Api<TaskInfo> {
    Ok(Json(app.hub.run_macro(&id, q).await?))
}
async fn control(
    State(app): State<App>,
    Path(id): Path<String>,
    Json(q): Json<ControlRequest>,
) -> Api<serde_json::Value> {
    app.hub.session(&id).await?.control(q).await?;
    Ok(Json(serde_json::json!({"ok":true})))
}
async fn takeover(State(app): State<App>, Path(id): Path<String>) -> Api<serde_json::Value> {
    app.hub.takeover(&id).await?;
    Ok(Json(serde_json::json!({"ok":true})))
}
#[derive(serde::Deserialize)]
struct TaskListQuery {
    session_id: Option<String>,
}
async fn tasks(State(app): State<App>, Query(query): Query<TaskListQuery>) -> Json<Vec<TaskInfo>> {
    Json(app.hub.task_list_for(query.session_id.as_deref()).await)
}
async fn task(State(app): State<App>, Path(id): Path<String>) -> Api<TaskInfo> {
    Ok(Json(app.hub.task(&id).await?))
}
async fn cancel(State(app): State<App>, Path(id): Path<String>) -> Api<TaskInfo> {
    Ok(Json(app.hub.cancel_task(&id).await?))
}

async fn stream(
    State(app): State<App>,
    Path(id): Path<String>,
    Query(q): Query<EventQuery>,
    ws: WebSocketUpgrade,
) -> std::result::Result<Response, ApiError> {
    let session = app.hub.session(&id).await?;
    Ok(ws.on_upgrade(move |mut socket| async move {
        let mut cursor = q.after;
        let mut interval = tokio::time::interval(Duration::from_millis(100));
        loop {
            tokio::select! {
                message = socket.recv() => match message { None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break, _ => {} },
                _ = interval.tick() => {
                    let page = session.page(&EventQuery { after:cursor, limit:Some(200), contains:None }).await;
                    if !page.events.is_empty() || page.gap {
                        cursor = page.next_cursor;
                        let text = match serde_json::to_string(&page) { Ok(s)=>s,Err(_)=>break };
                        if !matches!(tokio::time::timeout(Duration::from_secs(5),socket.send(Message::Text(text.into()))).await,Ok(Ok(_))) { break; }
                    }
                    if session.closed.load(std::sync::atomic::Ordering::Acquire) && !page.has_more { let _ = socket.close().await; break; }
                }
            }
        }
    }))
}

// Stream only the generated session's raw RX files, oldest retained rotation first.
async fn export(
    State(app): State<App>,
    Path(id): Path<String>,
    _headers: HeaderMap,
) -> std::result::Result<Response, ApiError> {
    let dir = app.hub.session_dir(&id)?;
    if !dir.join("session.json").is_file() {
        return Err(anyhow::anyhow!("session not found").into());
    }
    if let Ok(session) = app.hub.session(&id).await {
        session.persist().await?;
    }
    let mut files = Vec::new();
    for i in (0..LOG_FILES).rev() {
        let name = if i == 0 {
            "rx.log".into()
        } else {
            format!("rx.log.{i}")
        };
        match tokio::fs::File::open(dir.join(name)).await {
            Ok(f) => files.push(f),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    let stream = futures::stream::try_unfold(
        (files.into_iter(), None::<tokio::fs::File>),
        |(mut files, mut current)| async move {
            loop {
                if current.is_none() {
                    current = files.next();
                }
                let Some(file) = current.as_mut() else {
                    return Ok::<_, std::io::Error>(None);
                };
                let mut bytes = vec![0; 65536];
                let n = file.read(&mut bytes).await?;
                if n == 0 {
                    current = None;
                    continue;
                }
                bytes.truncate(n);
                return Ok(Some((bytes, (files, current))));
            }
        },
    );
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream".into()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{id}-rx.log\""),
            ),
        ],
        Body::from_stream(stream),
    )
        .into_response())
}

async fn shutdown(State(app): State<App>) -> Json<serde_json::Value> {
    app.hub.shutdown_requested.notify_one();
    Json(serde_json::json!({"stopping": true}))
}

async fn release_control(State(app): State<App>, Path(id): Path<String>) -> Api<serde_json::Value> {
    app.hub.release_control(&id).await?;
    Ok(Json(serde_json::json!({"ok": true})))
}

async fn task_details(State(app): State<App>, Path(id): Path<String>) -> Api<TaskDetails> {
    Ok(Json(app.hub.task_details(&id).await?))
}
async fn scripts(State(app): State<App>) -> Json<Vec<SavedScript>> {
    Json(app.hub.script_list().await)
}
async fn script(State(app): State<App>, Path(id): Path<String>) -> Api<SavedScript> {
    Ok(Json(app.hub.script(&id).await?))
}
async fn save_script(
    State(app): State<App>,
    Json(request): Json<SaveScriptRequest>,
) -> Api<SavedScript> {
    Ok(Json(app.hub.save_script(request).await?))
}
async fn delete_script(
    State(app): State<App>,
    Path(id): Path<String>,
    Json(request): Json<DeleteScriptRequest>,
) -> Api<serde_json::Value> {
    app.hub.delete_script(&id, request.revision).await?;
    Ok(Json(serde_json::json!({"deleted":true})))
}
async fn run_script(
    State(app): State<App>,
    Path(id): Path<String>,
    Json(request): Json<RunScriptRequest>,
) -> Api<TaskInfo> {
    Ok(Json(app.hub.run_script(&id, request).await?))
}
