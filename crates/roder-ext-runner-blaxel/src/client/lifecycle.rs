use anyhow::Context;
use serde_json::{Value, json};

use super::{BlaxelClient, Sandbox, decode};
use crate::config::SandboxLifecycle;

impl BlaxelClient {
    /// Reconcile lifecycle policy on an existing sandbox without replacing it.
    /// Blaxel's update endpoint accepts a writable sandbox document, so fetch
    /// and preserve the current spec while excluding read-only response fields.
    pub async fn update_sandbox_lifecycle(
        &self,
        name: &str,
        lifecycle: &SandboxLifecycle,
    ) -> anyhow::Result<Sandbox> {
        let path = format!("sandboxes/{}", urlencoding::encode(name));
        let response = self
            .control(reqwest::Method::GET, &path)
            .send()
            .await
            .context("get blaxel sandbox before lifecycle update")?;
        let document: Value = decode(response, "get sandbox before lifecycle update").await?;
        let update = lifecycle_update_document(&document, name, lifecycle)?;

        let response = self
            .control(reqwest::Method::PUT, &path)
            .json(&update)
            .send()
            .await
            .context("update blaxel sandbox lifecycle")?;
        decode(response, "update sandbox lifecycle").await
    }
}

fn lifecycle_update_document(
    document: &Value,
    name: &str,
    lifecycle: &SandboxLifecycle,
) -> anyhow::Result<Value> {
    let root = document
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("blaxel sandbox document must be an object"))?;
    let source_metadata = root.get("metadata").and_then(Value::as_object);
    let mut metadata = serde_json::Map::from_iter([("name".to_string(), json!(name))]);
    for key in ["displayName", "externalId", "labels"] {
        if let Some(value) = source_metadata
            .and_then(|source| source.get(key))
            .filter(|value| !value.is_null())
        {
            metadata.insert(key.to_string(), value.clone());
        }
    }

    let mut spec = root
        .get("spec")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("blaxel sandbox document is missing `spec`"))?;
    let spec_object = spec
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("blaxel sandbox `spec` must be an object"))?;
    spec_object.insert("lifecycle".to_string(), lifecycle.api_value());
    Ok(json!({ "metadata": metadata, "spec": spec }))
}
