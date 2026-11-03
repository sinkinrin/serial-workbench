use super::{model::*, storage, Hub};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::Arc};
use tokio::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedScript {
    #[serde(default = "new_id")]
    pub id: String,
    pub name: String,
    pub commands: String,
    #[serde(default = "default_newline")]
    pub newline: String,
    #[serde(default = "default_interval")]
    pub interval_ms: u64,
    #[serde(default)]
    pub wait_for: String,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub updated_by: String,
}
fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn default_newline() -> String {
    "crlf".into()
}
fn default_interval() -> u64 {
    200
}
fn default_timeout() -> u64 {
    5000
}

#[derive(Deserialize)]
pub struct SaveScriptRequest {
    #[serde(flatten)]
    pub script: SavedScript,
    #[serde(default = "source")]
    pub source: String,
}
#[derive(Deserialize)]
pub struct DeleteScriptRequest {
    pub revision: u64,
}
#[derive(Deserialize)]
pub struct RunScriptRequest {
    pub session_id: String,
    #[serde(default = "source")]
    pub source: String,
    #[serde(default)]
    pub revision: Option<u64>,
}
fn source() -> String {
    "cli".into()
}

impl SavedScript {
    fn lines(&self) -> Vec<&str> {
        self.commands
            .split(['\r', '\n'])
            .filter(|line| !line.trim().is_empty())
            .collect()
    }
    pub fn validate(&self) -> Result<()> {
        uuid::Uuid::parse_str(&self.id).context("invalid script id")?;
        if self.name.trim().is_empty() || self.name.chars().count() > 64 {
            bail!("script name requires 1..64 characters");
        }
        if self.commands.chars().count() > 65536
            || self.lines().is_empty()
            || self.lines().len() > 100
        {
            bail!("script requires 1..100 commands, at most 65536 characters");
        }
        if !["crlf", "lf", "cr", "none"].contains(&self.newline.as_str()) {
            bail!("newline must be crlf, lf, cr or none");
        }
        if self.interval_ms > 60000
            || !(1..=300000).contains(&self.timeout_ms)
            || self.wait_for.chars().count() > 256
        {
            bail!("invalid script timing or response text");
        }
        Ok(())
    }
    fn macro_request(&self, source: String) -> MacroRequest {
        let ending = match self.newline.as_str() {
            "crlf" => "\r\n",
            "lf" => "\n",
            "cr" => "\r",
            _ => "",
        };
        let lines = self.lines();
        let mut steps = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            steps.push(serde_json::json!({"type":"send","data":format!("{line}{ending}"),"encoding":"utf8"}));
            if !self.wait_for.is_empty() {
                steps.push(serde_json::json!({"type":"expect","op":"contains","data":self.wait_for,"encoding":"utf8","timeout_ms":self.timeout_ms,"idle_ms":100,"max_bytes":1048576,"trim":false}));
            }
            if self.interval_ms > 0 && index + 1 < lines.len() {
                steps.push(serde_json::json!({"type":"delay","ms":self.interval_ms}));
            }
        }
        MacroRequest {
            name: self.name.trim().into(),
            source,
            assembly: false,
            pack: serde_json::json!({"schema_version":"0.3","name":"workbench-shared-scripts","macros":[{"name":self.name.trim(),"steps":steps}],"assemblies":[]}),
        }
    }
}

pub async fn load(path: &Path) -> Result<Vec<SavedScript>> {
    match fs::read(path).await {
        Ok(bytes) => {
            let scripts: Vec<SavedScript> = serde_json::from_slice(&bytes)
                .context("scripts.json is invalid; preserved without overwriting")?;
            for script in &scripts {
                script.validate()?;
            }
            Ok(scripts)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.into()),
    }
}
impl Hub {
    pub async fn script_list(&self) -> Vec<SavedScript> {
        self.scripts.lock().await.clone()
    }
    pub async fn script(&self, id: &str) -> Result<SavedScript> {
        self.scripts
            .lock()
            .await
            .iter()
            .find(|s| s.id == id)
            .cloned()
            .context("script not found")
    }
    pub async fn save_script(&self, mut request: SaveScriptRequest) -> Result<SavedScript> {
        request.script.validate()?;
        if request.source.is_empty() || request.source.len() > 64 {
            bail!("invalid source");
        }
        let mut scripts = self.scripts.lock().await;
        let index = scripts.iter().position(|s| s.id == request.script.id);
        if let Some(index) = index {
            if scripts[index].revision != request.script.revision {
                bail!("script was changed by another client; reload before saving");
            }
        } else if request.script.revision != 0 {
            bail!("script was deleted; reload before saving");
        } else if scripts.len() >= 100 {
            bail!("maximum 100 shared scripts");
        }
        request.script.revision = index.map(|i| scripts[i].revision).unwrap_or(0) + 1;
        request.script.updated_at = now();
        request.script.updated_by = request.source;
        request.script.name = request.script.name.trim().into();
        let mut next = scripts.clone();
        if let Some(index) = index {
            next[index] = request.script.clone();
        } else {
            next.push(request.script.clone());
        }
        storage::write_json(&self.data_dir.join("scripts.json"), &next).await?;
        *scripts = next;
        Ok(request.script)
    }
    pub async fn delete_script(&self, id: &str, revision: u64) -> Result<()> {
        let mut scripts = self.scripts.lock().await;
        let existing = scripts
            .iter()
            .find(|s| s.id == id)
            .context("script not found")?;
        if existing.revision != revision {
            bail!("script was changed by another client; reload before deleting");
        }
        let next: Vec<_> = scripts.iter().filter(|s| s.id != id).cloned().collect();
        storage::write_json(&self.data_dir.join("scripts.json"), &next).await?;
        *scripts = next;
        Ok(())
    }
    pub async fn run_script(
        self: &Arc<Self>,
        id: &str,
        request: RunScriptRequest,
    ) -> Result<TaskInfo> {
        let script = self.script(id).await?;
        if request
            .revision
            .is_some_and(|revision| revision != script.revision)
        {
            bail!("script was changed; reload and review it before running");
        }
        self.run_macro_from_script(
            &request.session_id,
            script.macro_request(request.source),
            Some(ScriptReference {
                id: script.id,
                revision: script.revision,
            }),
        )
        .await
    }
}
