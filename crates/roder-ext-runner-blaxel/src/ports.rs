use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A sandbox port declared when its runtime is first created.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxPort {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub target: u16,
    pub protocol: PortProtocol,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum PortProtocol {
    Http,
    Tcp,
}

pub(crate) fn parse_ports(config: &Value) -> anyhow::Result<Vec<SandboxPort>> {
    let Some(value) = config.get("ports") else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    let ports: Vec<SandboxPort> =
        serde_json::from_value(value.clone()).context("parse blaxel runner `ports`")?;
    anyhow::ensure!(
        ports.len() <= 16,
        "blaxel runner cannot declare more than 16 ports"
    );
    for (index, port) in ports.iter().enumerate() {
        anyhow::ensure!(
            port.target != 0 && ![80, 443, 8080].contains(&port.target),
            "blaxel runner port must be nonzero and cannot use reserved ports 80, 443 or 8080"
        );
        anyhow::ensure!(
            !ports[..index]
                .iter()
                .any(|other| other.target == port.target),
            "blaxel runner contains a duplicate port target"
        );
        if let Some(name) = &port.name {
            anyhow::ensure!(
                !name.is_empty()
                    && name.len() <= 49
                    && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
                "blaxel runner port name must use 1 to 49 ASCII letters, digits or hyphens"
            );
            anyhow::ensure!(
                !ports[..index]
                    .iter()
                    .any(|other| other.name.as_ref() == Some(name)),
                "blaxel runner contains a duplicate port name"
            );
        }
    }
    Ok(ports)
}

#[cfg(test)]
mod tests {
    use super::parse_ports;
    use serde_json::json;

    #[test]
    fn ports_reject_invalid_or_ambiguous_runtime_routes() {
        for value in [
            json!([{ "target": 0, "protocol": "HTTP" }]),
            json!([{ "target": 8080, "protocol": "HTTP" }]),
            json!([{ "target": 65536, "protocol": "HTTP" }]),
            json!([{ "target": 4319, "protocol": "UDP" }]),
            json!([{ "target": 4319, "protocol": "HTTP", "public": true }]),
            json!([{ "target": 4319, "protocol": "HTTP", "name": "" }]),
            json!([{ "target": 4319, "protocol": "HTTP" }, { "target": 4319, "protocol": "TCP" }]),
            json!([{ "target": 4319, "protocol": "HTTP", "name": "same" }, { "target": 4320, "protocol": "HTTP", "name": "same" }]),
        ] {
            assert!(
                parse_ports(&json!({ "ports": value })).is_err(),
                "accepted {value}"
            );
        }
    }

    #[test]
    fn optional_ports_allow_absence_null_and_an_empty_list() {
        for config in [json!({}), json!({ "ports": null }), json!({ "ports": [] })] {
            assert!(parse_ports(&config).unwrap().is_empty());
        }
        let too_many = vec![json!({ "target": 4319, "protocol": "HTTP" }); 17];
        assert!(
            parse_ports(&json!({ "ports": too_many }))
                .unwrap_err()
                .to_string()
                .contains("16 ports")
        );
    }
}
