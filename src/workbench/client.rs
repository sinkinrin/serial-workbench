use anyhow::{bail, Context, Result};
use reqwest::{Client, Method};
use serde_json::Value;

#[derive(Clone)]
pub struct WorkbenchClient {
    client: Client,
    base: String,
}
impl WorkbenchClient {
    pub fn new(base: &str) -> Result<Self> {
        let parsed = reqwest::Url::parse(base)?;
        if parsed.scheme() != "http"
            || !matches!(parsed.host_str(), Some("127.0.0.1" | "localhost"))
        {
            bail!("workbench URL must be local HTTP (127.0.0.1 or localhost)");
        }
        Ok(Self {
            client: Client::builder()
                .no_proxy()
                .connect_timeout(std::time::Duration::from_secs(3))
                .timeout(std::time::Duration::from_secs(30))
                .build()?,
            base: base.trim_end_matches('/').into(),
        })
    }
    pub async fn request(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        let mut request = self
            .client
            .request(method, format!("{}{path}", self.base))
            .header("x-serial-workbench", "1");
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .context("cannot reach workbench; start `serial-workbench serve` first")?;
        let status = response.status();
        let body: Value = response
            .json()
            .await
            .context("invalid workbench response")?;
        if !status.is_success() {
            bail!(
                "{}",
                body.get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("workbench request rejected")
            );
        }
        Ok(body)
    }
    pub async fn get(&self, path: &str) -> Result<Value> {
        self.request(Method::GET, path, None).await
    }
    pub async fn post(&self, path: &str, body: Value) -> Result<Value> {
        self.request(Method::POST, path, Some(body)).await
    }
    pub async fn wait_task(&self, id: &str) -> Result<Value> {
        loop {
            let task = self.get(&format!("/api/tasks/{id}")).await?;
            if task["finished_at"].is_string() {
                return Ok(task);
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
}
