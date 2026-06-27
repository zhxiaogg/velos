//! A thin REST client the worker uses to talk to the apiserver.
//!
//! Wire payloads are opaque `serde_json::Value`: the worker reads only the
//! envelope fields it needs and writes status through the `/status` subresource,
//! preserving the spec/status split.

use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("http error: {0}")]
    Http(String),
    #[error("apiserver returned {status}: {body}")]
    Status { status: u16, body: String },
}

impl From<reqwest::Error> for ClientError {
    fn from(e: reqwest::Error) -> Self {
        ClientError::Http(e.to_string())
    }
}

/// REST client bound to one apiserver base URL, optionally bearer-authenticated.
#[derive(Clone)]
pub struct ApiClient {
    base: String,
    http: reqwest::Client,
    token: Option<String>,
}

impl ApiClient {
    pub fn new(base: impl Into<String>, token: Option<String>) -> Self {
        Self {
            base: base.into(),
            http: reqwest::Client::new(),
            token,
        }
    }

    fn auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.token {
            Some(t) => rb.bearer_auth(t),
            None => rb,
        }
    }

    async fn send(&self, rb: reqwest::RequestBuilder) -> Result<Value, ClientError> {
        let resp = self.auth(rb).send().await?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(ClientError::Status {
                status: status.as_u16(),
                body: text,
            });
        }
        if text.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).map_err(|e| ClientError::Http(e.to_string()))
    }

    /// List containers assigned to `node` (`fieldSelector=spec.nodeName=node`).
    pub async fn list_assigned(&self, node: &str) -> Result<Vec<Value>, ClientError> {
        let url = format!(
            "{}/api/v1/containers?fieldSelector=spec.nodeName={node}",
            self.base
        );
        let body = self.send(self.http.get(url)).await?;
        Ok(body
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    pub async fn get_container(&self, name: &str) -> Result<Option<Value>, ClientError> {
        let url = format!("{}/api/v1/containers/{name}", self.base);
        match self.send(self.http.get(url)).await {
            Ok(v) => Ok(Some(v)),
            Err(ClientError::Status { status: 404, .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Write a container's status through the status subresource.
    pub async fn put_status(&self, name: &str, status: Value) -> Result<Value, ClientError> {
        let url = format!("{}/api/v1/containers/{name}/status", self.base);
        self.send(
            self.http
                .put(url)
                .json(&serde_json::json!({ "status": status })),
        )
        .await
    }

    /// Replace a container (used to clear our finalizer).
    pub async fn replace_container(&self, name: &str, body: &Value) -> Result<Value, ClientError> {
        let url = format!("{}/api/v1/containers/{name}", self.base);
        self.send(self.http.put(url).json(body)).await
    }

    /// Create-or-renew this worker's lease (heartbeat).
    pub async fn renew_lease(&self, node: &str, duration_secs: u32) -> Result<(), ClientError> {
        let now = chrono::Utc::now().to_rfc3339();
        let spec = serde_json::json!({
            "holderIdentity": node,
            "renewTime": now,
            "leaseDurationSeconds": duration_secs,
        });
        let existing = {
            let url = format!("{}/api/v1/leases/{node}", self.base);
            match self.send(self.http.get(url)).await {
                Ok(v) => Some(v),
                Err(ClientError::Status { status: 404, .. }) => None,
                Err(e) => return Err(e),
            }
        };
        if existing.is_some() {
            let url = format!("{}/api/v1/leases/{node}", self.base);
            let body = serde_json::json!({ "metadata": { "name": node }, "spec": spec });
            self.send(self.http.put(url).json(&body)).await?;
        } else {
            let url = format!("{}/api/v1/leases", self.base);
            let body = serde_json::json!({ "metadata": { "name": node }, "spec": spec });
            self.send(self.http.post(url).json(&body)).await?;
        }
        Ok(())
    }

    /// Join the cluster with a bootstrap token, receiving a worker credential.
    pub async fn register(&self, request: &Value) -> Result<Value, ClientError> {
        let url = format!("{}/auth/v1/register", self.base);
        self.send(self.http.post(url).json(request)).await
    }
}
