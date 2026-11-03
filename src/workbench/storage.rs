//! Small local JSON stores shared by the GUI, CLI and MCP adapter.
use anyhow::{Context, Result};
use serde::Serialize;
use std::path::Path;
use tokio::fs;

pub async fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    fs::write(&temp, serde_json::to_vec_pretty(value)?).await?;
    if let Err(error) = fs::rename(&temp, path).await {
        let _ = fs::remove_file(&temp).await;
        return Err(error).with_context(|| format!("cannot save {}", path.display()));
    }
    Ok(())
}
