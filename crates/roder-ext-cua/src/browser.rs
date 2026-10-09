//! Browser capabilities share the native desktop's thread/runner session and fence.
use crate::{CuaConfig, CuaTarget, CuaTransport, LocalDesktopLease};
use roder_api::tools::{ToolCall, ToolResult};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(crate) struct BrowserSession {
    started: bool,
    targets: HashMap<String, HashSet<String>>,
    snapshots: HashMap<(String, String), Snapshot>,
}
struct Snapshot {
    refs: HashMap<String, HashSet<String>>,
    generation: Option<u64>,
}
impl BrowserSession {
    pub(crate) fn invalidate(&mut self) {
        self.snapshots.clear();
    }
    fn check_tab(&self, args: &Value) -> anyhow::Result<(String, String)> {
        let target = args["target_id"].as_str().unwrap_or_default();
        let tab = args["tab_id"].as_str().unwrap_or_default();
        anyhow::ensure!(
            self.targets
                .get(target)
                .is_some_and(|tabs| tabs.contains(tab)),
            "browser target/tab is not bound to this thread and runner; bind the native window again"
        );
        Ok((target.into(), tab.into()))
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run(
    name: &str,
    mut args: Value,
    call: &ToolCall,
    session: &mut BrowserSession,
    config: &CuaConfig,
    transport: &dyn CuaTransport,
    destination: CuaTarget<'_>,
    label: &str,
    timeout: u64,
    lease: Option<&LocalDesktopLease>,
) -> anyhow::Result<ToolResult> {
    if name == "end_browser_session" {
        session.started = false;
        session.targets.clear();
        session.invalidate();
        if let Some(lease) = lease {
            lease.begin_input();
        }
        let reply = transport
            .call(
                destination,
                "end_session",
                json!({"session":label}),
                timeout,
            )
            .await?;
        let data = json!({"untrusted":true,"observation":reply.observation});
        return Ok(ToolResult {
            id: call.id.clone(),
            name: call.name.clone(),
            text: crate::render::result_text(&data),
            data,
            is_error: reply.is_error,
        });
    }
    let input = crate::browser_specs::is_input(name);
    let bind = name == "get_browser_state" && args.get("target_id").is_none();
    let key = if name == "browser_prepare" || bind {
        None
    } else {
        Some(session.check_tab(&args)?)
    };
    if bind {
        anyhow::ensure!(
            args.get("pid").is_some()
                && args.get("window_id").is_some()
                && args.get("tab_id").is_none()
                && args.get("scope_ref").is_none()
                && args.get("query").is_none()
                && args.get("continuation").is_none(),
            "bind requires only pid/window_id; snapshot requires target_id/tab_id"
        );
    } else if name != "browser_prepare" {
        anyhow::ensure!(
            args.get("pid").is_none() && args.get("window_id").is_none(),
            "cannot combine native bind and browser snapshot targets"
        );
    }
    if let Some(key) = &key {
        if let Some(reference) = args.get("ref").and_then(Value::as_str) {
            let snapshot = session
                .snapshots
                .get(key)
                .ok_or_else(|| anyhow::anyhow!("snapshot this tab before browser input"))?;
            anyhow::ensure!(
                snapshot.generation == lease.map(LocalDesktopLease::generation),
                "another local thread changed the desktop; snapshot this tab again"
            );
            let action = if name == "browser_click" {
                "click"
            } else {
                "type"
            };
            anyhow::ensure!(
                snapshot
                    .refs
                    .get(reference)
                    .is_some_and(|actions| actions.contains(action)),
                "browser ref is stale, foreign, or does not declare this action; snapshot again"
            );
        }
        // Clear before dispatch, including failed snapshots/cancelled mutations.
        session.snapshots.remove(key);
    }
    if name == "browser_prepare" {
        let mode = args["profile_mode"].as_str().unwrap().to_owned();
        if mode == "existing_profile" {
            anyhow::ensure!(
                config.allow_existing_browser_profile,
                "existing browser profile attachment requires cua.allow_existing_browser_profile=true and a driver launch-time existing-profile grant"
            );
            anyhow::ensure!(
                args.get("profile_name").is_none(),
                "existing_profile cannot name a driver-owned profile"
            );
            args["strategy"] = json!({"kind":"existing_profile"});
        } else {
            let mut profile = json!({"mode":mode});
            if mode == "isolated_named" {
                let name = args["profile_name"].as_str().unwrap_or_default();
                anyhow::ensure!(
                    !name.is_empty()
                        && name.len() <= 64
                        && name
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
                    "isolated_named requires a path-safe profile_name"
                );
                profile["name"] = json!(name);
            } else {
                anyhow::ensure!(
                    args.get("profile_name").is_none(),
                    "isolated_new cannot have profile_name"
                );
            }
            args["profile"] = profile;
            args["allow_launch"] = json!(true);
        }
        args.as_object_mut().unwrap().remove("profile_mode");
        args.as_object_mut().unwrap().remove("profile_name");
        session.targets.clear();
        session.invalidate();
    }
    if input {
        session.invalidate();
        if let Some(lease) = lease {
            lease.begin_input();
        }
    }
    args["session"] = json!(label);
    if name == "get_browser_state" && !bind {
        args["snapshot_format"] = json!("semantic_v2");
        args["include_screenshot"] = json!(true);
    }
    if !session.started {
        let reply = transport
            .call(
                destination,
                "start_session",
                json!({"session":label}),
                timeout,
            )
            .await?;
        anyhow::ensure!(
            !reply.is_error,
            "Cua browser session could not start: {}",
            reply.observation
        );
        session.started = true;
    }
    let mut data = json!({"untrusted":true,"driver_version":crate::DRIVER_VERSION});
    let mut failed;
    match transport.call(destination, name, args, timeout).await {
        Ok(mut reply) => {
            failed = reply.is_error || reply.observation["status"] == "refused";
            if !failed && bind {
                let target = reply.observation["target_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("browser bind returned no target"))?;
                if reply.observation["binding_quality"] == "exact" {
                    let tabs = reply.observation["tabs"]
                        .as_array()
                        .ok_or_else(|| anyhow::anyhow!("browser bind returned no tabs"))?;
                    if session.targets.len() >= 32 {
                        session.targets.clear();
                        session.invalidate();
                    }
                    session.targets.insert(
                        target.into(),
                        tabs.iter()
                            .filter_map(|tab| tab["tab_id"].as_str().map(str::to_owned))
                            .collect(),
                    );
                }
            } else if !failed && name == "get_browser_state" {
                snapshot(
                    &mut data,
                    &mut reply.observation,
                    session,
                    key.as_ref().unwrap(),
                    label,
                    lease,
                )?;
            }
            data["observation"] = reply.observation;
        }
        Err(error) => {
            failed = true;
            data["errors"] = json!([error.to_string()]);
        }
    }
    if input && name != "browser_prepare" {
        let key = key.as_ref().unwrap();
        // Observation is safe after refusal/uncertain dispatch; never replay the action.
        let reply = transport.call(destination, "get_browser_state", json!({"session":label,
            "target_id":key.0,"tab_id":key.1,"snapshot_format":"semantic_v2","include_screenshot":true}), timeout).await;
        match reply {
            Ok(mut reply) if !reply.is_error && reply.observation["status"] != "refused" => {
                if let Err(error) = snapshot(
                    &mut data,
                    &mut reply.observation,
                    session,
                    key,
                    label,
                    lease,
                ) {
                    failed = true;
                    data["capture_error"] = json!(error.to_string());
                }
                data["after_action"] = reply.observation;
            }
            _ => {
                failed = true;
                data["capture_error"] = json!(
                    "Browser after-action capture failed; snapshot or bind again. No input was retried."
                );
            }
        }
    }
    Ok(ToolResult {
        id: call.id.clone(),
        name: call.name.clone(),
        text: crate::render::result_text(&data),
        data,
        is_error: failed,
    })
}

fn snapshot(
    data: &mut Value,
    observation: &mut Value,
    session: &mut BrowserSession,
    key: &(String, String),
    label: &str,
    lease: Option<&LocalDesktopLease>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        observation["target_id"] == key.0 && observation["tab_id"] == key.1,
        "browser snapshot returned another target/tab"
    );
    // Browser PNGs have viewport metadata rather than native capture ids.
    // This id validates media only and never authorizes native pixel input.
    observation["capture_id"] = json!(format!("roder-browser-{}", uuid::Uuid::new_v4()));
    let mut capture = crate::capture::take_capture(observation, label)?;
    data[roder_api::transcript::VIEW_IMAGE_DISPLAY_KEY] = capture.image.take();
    let refs = observation["refs"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("browser snapshot returned no semantic refs"))?;
    session.snapshots.insert(
        key.clone(),
        Snapshot {
            generation: lease.map(LocalDesktopLease::generation),
            refs: refs
                .iter()
                .filter_map(|row| {
                    Some((
                        row["ref"].as_str()?.to_owned(),
                        row["actions"]
                            .as_array()?
                            .iter()
                            .filter_map(|action| action.as_str().map(str::to_owned))
                            .collect(),
                    ))
                })
                .collect(),
        },
    );
    Ok(())
}
