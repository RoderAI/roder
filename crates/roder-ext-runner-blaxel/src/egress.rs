use anyhow::Context;
use serde_json::Value;

use crate::{BlaxelConfig, client::Sandbox};

pub(crate) fn gateway(config: &Value) -> anyhow::Result<Option<String>> {
    let Some(value) = config
        .get("egress_gateway")
        .filter(|value| !value.is_null())
    else {
        return Ok(None);
    };
    let gateway = value
        .as_str()
        .context("blaxel egress gateway must be a string")?;
    anyhow::ensure!(
        !gateway.is_empty()
            && gateway.len() <= 63
            && gateway
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            && !gateway.starts_with('-')
            && !gateway.ends_with('-'),
        "blaxel egress gateway must be a DNS label"
    );
    let region = config.get("region").and_then(Value::as_str);
    anyhow::ensure!(
        region.is_some_and(|region| !region.trim().is_empty() && region.trim() != "auto"),
        "dedicated egress requires a concrete sandbox region"
    );
    Ok(Some(gateway.to_owned()))
}

// createIfNotExist and external-id recovery can return an older machine. Check
// the provider's immutable assignment before any workload operation.
pub(crate) fn verify(sandbox: &Sandbox, config: &BlaxelConfig) -> anyhow::Result<()> {
    let egress = &sandbox.spec["network"]["egress"];
    if let Some(gateway) = &config.egress_gateway {
        anyhow::ensure!(
            egress["mode"] == "dedicated"
                && egress["gateway"] == gateway.as_str()
                && sandbox.spec["region"].as_str() == config.region.as_deref(),
            "sandbox does not match its dedicated egress gateway and region"
        );
    } else {
        anyhow::ensure!(
            egress["mode"] != "dedicated",
            "shared-egress runner cannot reuse a dedicated-egress sandbox"
        );
    }
    Ok(())
}
