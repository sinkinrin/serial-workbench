use super::client::WorkbenchClient;
use rmcp::{
    handler::server::{router::tool::ToolRouter, tool::Parameters},
    model::*,
    tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler, ServiceExt,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use std::future::Future;

#[derive(Clone)]
pub struct WorkbenchMcp {
    client: WorkbenchClient,
    tool_router: ToolRouter<Self>,
}
#[derive(Deserialize, JsonSchema)]
pub struct SessionId {
    pub session_id: String,
}
#[derive(Deserialize, JsonSchema)]
pub struct TaskId {
    pub task_id: String,
}
#[derive(Deserialize, JsonSchema)]
pub struct TaskDetailsArgs {
    pub task_id: String,
    #[serde(default = "yes")]
    pub details: bool,
}
fn yes() -> bool {
    true
}
#[derive(Deserialize, JsonSchema)]
pub struct ScriptId {
    pub script_id: String,
}
#[derive(Deserialize, JsonSchema)]
pub struct ScriptSave {
    pub script: Value,
}
#[derive(Deserialize, JsonSchema)]
pub struct ScriptDelete {
    pub script_id: String,
    pub revision: u64,
}
#[derive(Deserialize, JsonSchema)]
pub struct ScriptRun {
    pub script_id: String,
    pub session_id: String,
    #[serde(default)]
    pub revision: Option<u64>,
}
#[derive(Deserialize, JsonSchema)]
pub struct Open {
    pub port: String,
    #[serde(default = "baud")]
    pub baud_rate: u32,
    #[serde(default)]
    pub demo: bool,
    #[serde(default)]
    pub data_bits: Option<String>,
    #[serde(default)]
    pub stop_bits: Option<String>,
    #[serde(default)]
    pub parity: Option<String>,
    #[serde(default)]
    pub flow_control: Option<String>,
}
fn baud() -> u32 {
    115200
}
#[derive(Deserialize, JsonSchema)]
pub struct Events {
    pub session_id: String,
    #[serde(default)]
    pub after: u64,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub contains: Option<String>,
    #[serde(default)]
    pub history: bool,
}
#[derive(Deserialize, JsonSchema)]
pub struct SendArgs {
    pub session_id: String,
    pub data: String,
    #[serde(default)]
    pub encoding: Option<String>,
    #[serde(default)]
    pub wait_for: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub sensitive: bool,
}
#[derive(Deserialize, JsonSchema)]
pub struct Macro {
    pub session_id: String,
    pub pack: Value,
    pub name: String,
    #[serde(default)]
    pub assembly: bool,
}
#[derive(Deserialize, JsonSchema)]
pub struct Control {
    pub session_id: String,
    pub rts: Option<bool>,
    pub dtr: Option<bool>,
}
fn error(e: impl std::fmt::Display) -> McpError {
    McpError::invalid_params(e.to_string(), None)
}
fn result(value: Value) -> Result<CallToolResult, McpError> {
    Ok(CallToolResult::success(vec![Content::text(
        value.to_string(),
    )]))
}
fn id(value: &str) -> Result<String, McpError> {
    uuid::Uuid::parse_str(value)
        .map(|v| v.to_string())
        .map_err(error)
}

#[tool_router]
impl WorkbenchMcp {
    pub fn new(client: WorkbenchClient) -> Self {
        Self {
            client,
            tool_router: Self::tool_router(),
        }
    }
    #[tool(
        description = "List OS serial ports. Opening is explicit; no port is opened by this query."
    )]
    async fn serial_ports(&self) -> Result<CallToolResult, McpError> {
        result(self.client.get("/api/ports").await.map_err(error)?)
    }
    #[tool(
        description = "List shared active and historical sessions with parameters, byte counts and local log directories. Attach to existing IDs rather than reopening a port."
    )]
    async fn serial_sessions(&self) -> Result<CallToolResult, McpError> {
        result(self.client.get("/api/sessions").await.map_err(error)?)
    }
    #[tool(
        description = "Open one shared serial session in the independent daemon. Capture persists when this MCP client exits. demo=true creates a simulated device without touching hardware."
    )]
    async fn serial_open(
        &self,
        Parameters(a): Parameters<Open>,
    ) -> Result<CallToolResult, McpError> {
        result(self.client.post("/api/sessions",json!({"port":a.port,"baud_rate":a.baud_rate,"demo":a.demo,"data_bits":a.data_bits.unwrap_or("8".into()),"stop_bits":a.stop_bits.unwrap_or("1".into()),"parity":a.parity.unwrap_or("none".into()),"flow_control":a.flow_control.unwrap_or("none".into())})).await.map_err(error)?)
    }
    #[tool(
        description = "Read shared events without consuming them. Pass next_cursor as after next time. gap means older data was evicted; history=true reads retained disk records. Search is per event chunk; use local rx.log files for full line/context searches."
    )]
    async fn serial_events(
        &self,
        Parameters(a): Parameters<Events>,
    ) -> Result<CallToolResult, McpError> {
        let mut url = reqwest::Url::parse("http://localhost/").unwrap();
        url.query_pairs_mut()
            .append_pair("after", &a.after.to_string())
            .append_pair("limit", &a.limit.unwrap_or(200).to_string());
        if let Some(s) = a.contains {
            url.query_pairs_mut().append_pair("contains", &s);
        }
        result(
            self.client
                .get(&format!(
                    "/api/sessions/{}/{}?{}",
                    id(&a.session_id)?,
                    if a.history { "history" } else { "events" },
                    url.query().unwrap()
                ))
                .await
                .map_err(error)?,
        )
    }
    #[tool(
        description = "Queue a send task. Data is sent exactly as provided; include CR/LF explicitly. Optional wait_for verifies a response within timeout_ms. Returns a task ID immediately; serial_task gives completion and event cursors. sensitive hides TX records only; device echo remains captured."
    )]
    async fn serial_send(
        &self,
        Parameters(a): Parameters<SendArgs>,
    ) -> Result<CallToolResult, McpError> {
        result(self.client.post(&format!("/api/sessions/{}/send",id(&a.session_id)?),json!({"data":a.data,"encoding":a.encoding.unwrap_or("utf8".into()),"source":"mcp","wait_for":a.wait_for,"timeout_ms":a.timeout_ms.unwrap_or(5000),"sensitive":a.sensitive})).await.map_err(error)?)
    }
    #[tool(
        description = "Get persisted task status, exact planned commands and a bounded RX/TX window by default (details=false gives status only). Unsolicited logs may be present; truncated/gap/finalized flags describe coverage. Sensitive input and its transcript are hidden. sent only confirms transmission; succeeded means configured expectations matched; completed means a macro finished without response expectations. failed/cancelled must not be treated as success."
    )]
    async fn serial_task(
        &self,
        Parameters(a): Parameters<TaskDetailsArgs>,
    ) -> Result<CallToolResult, McpError> {
        let path = format!(
            "/api/tasks/{}{}",
            id(&a.task_id)?,
            if a.details { "/details" } else { "" }
        );
        result(self.client.get(&path).await.map_err(error)?)
    }
    #[tool(
        description = "Cancel queued/running host task. Already transmitted bytes cannot be recalled. Does not send Ctrl-C or stop a program running on the device."
    )]
    async fn serial_cancel(
        &self,
        Parameters(a): Parameters<TaskId>,
    ) -> Result<CallToolResult, McpError> {
        result(
            self.client
                .post(&format!("/api/tasks/{}/cancel", id(&a.task_id)?), json!({}))
                .await
                .map_err(error)?,
        )
    }
    #[tool(
        description = "Run an existing JSON macro pack against a shared session. One task holds the send queue across the entire macro; recording and all viewers continue."
    )]
    async fn serial_macro(
        &self,
        Parameters(a): Parameters<Macro>,
    ) -> Result<CallToolResult, McpError> {
        result(
            self.client
                .post(
                    &format!("/api/sessions/{}/macro", id(&a.session_id)?),
                    json!({"pack":a.pack,"name":a.name,"assembly":a.assembly,"source":"mcp"}),
                )
                .await
                .map_err(error)?,
        )
    }
    #[tool(
        description = "Set RTS/DTR through the shared device owner. This changes physical control lines and may reset attached hardware; only use when requested."
    )]
    async fn serial_control(
        &self,
        Parameters(a): Parameters<Control>,
    ) -> Result<CallToolResult, McpError> {
        result(
            self.client
                .post(
                    &format!("/api/sessions/{}/control", id(&a.session_id)?),
                    json!({"rts":a.rts,"dtr":a.dtr,"source":"mcp"}),
                )
                .await
                .map_err(error)?,
        )
    }
    #[tool(
        description = "List shared saved command scripts, including IDs and revisions. These are the same scripts shown in the GUI and persisted in scripts.json."
    )]
    async fn serial_scripts(&self) -> Result<CallToolResult, McpError> {
        result(self.client.get("/api/scripts").await.map_err(error)?)
    }
    #[tool(description = "Read a shared script before running or editing it.")]
    async fn serial_script_get(
        &self,
        Parameters(a): Parameters<ScriptId>,
    ) -> Result<CallToolResult, McpError> {
        result(
            self.client
                .get(&format!("/api/scripts/{}", id(&a.script_id)?))
                .await
                .map_err(error)?,
        )
    }
    #[tool(
        description = "Create or update a shared script without executing it. script is JSON with name, commands (one command per line), optional id, revision, newline, intervalMs, waitFor, timeoutMs. Editing requires the latest revision. GUI and CLI see the same saved record."
    )]
    async fn serial_script_save(
        &self,
        Parameters(mut a): Parameters<ScriptSave>,
    ) -> Result<CallToolResult, McpError> {
        a.script
            .as_object_mut()
            .ok_or_else(|| error("script must be a JSON object"))?
            .insert("source".into(), json!("mcp"));
        result(
            self.client
                .post("/api/scripts", a.script)
                .await
                .map_err(error)?,
        )
    }
    #[tool(
        description = "Delete a saved shared script using its latest revision. Does not cancel an already queued execution snapshot."
    )]
    async fn serial_script_delete(
        &self,
        Parameters(a): Parameters<ScriptDelete>,
    ) -> Result<CallToolResult, McpError> {
        result(
            self.client
                .post(
                    &format!("/api/scripts/{}/delete", id(&a.script_id)?),
                    json!({"revision":a.revision}),
                )
                .await
                .map_err(error)?,
        )
    }
    #[tool(
        description = "Run a previously reviewed shared script against an existing session. Pass revision to ensure it has not changed. Returns task ID; serial_task includes commands and received bytes. This sends real device commands, so only run operations authorized by the user."
    )]
    async fn serial_script_run(
        &self,
        Parameters(a): Parameters<ScriptRun>,
    ) -> Result<CallToolResult, McpError> {
        result(
            self.client
                .post(
                    &format!("/api/scripts/{}/run", id(&a.script_id)?),
                    json!({"session_id":id(&a.session_id)?,"source":"mcp","revision":a.revision}),
                )
                .await
                .map_err(error)?,
        )
    }
    #[tool(
        description = "Explicitly close a shared physical session for ALL viewers and stop capture. To merely detach, just stop calling tools; do not close a session someone is using."
    )]
    async fn serial_close(
        &self,
        Parameters(a): Parameters<SessionId>,
    ) -> Result<CallToolResult, McpError> {
        result(
            self.client
                .post(
                    &format!("/api/sessions/{}/close", id(&a.session_id)?),
                    json!({}),
                )
                .await
                .map_err(error)?,
        )
    }
}
#[tool_handler]
impl ServerHandler for WorkbenchMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo { capabilities:ServerCapabilities::builder().enable_tools().build(),instructions:Some("Shared Serial Workbench adapter. The independent daemon owns ports. List sessions first and reuse them. Prefer local log files for large historical searches, cursor-based events for recent data, and one background task for multi-step work. Never close a session just to detach. Do not send device commands without user authorization.".into()),..Default::default() }
    }
}
pub async fn serve(client: WorkbenchClient) -> anyhow::Result<()> {
    let service = WorkbenchMcp::new(client)
        .serve(rmcp::transport::stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}
