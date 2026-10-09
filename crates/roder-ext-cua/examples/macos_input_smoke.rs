//! Opt-in live qualification on an explicitly owned AppKit fixture.
use base64::Engine;
use roder_api::{policy_mode::PolicyMode, tools::*};
use roder_ext_cua::{CuaBackend, CuaConfig, CuaToolContributor};
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};

struct Harness {
    registry: ToolRegistry,
    trace: Vec<Value>,
    output: PathBuf,
    state: PathBuf,
}
impl Harness {
    fn new(output: PathBuf, state: PathBuf) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&output)?;
        let mut registry = ToolRegistry::default();
        CuaToolContributor::new(CuaConfig {
            backend: CuaBackend::LocalMacos,
            max_image_dimension: 600,
            ..Default::default()
        })
        .contribute(&mut registry)?;
        Ok(Self {
            registry,
            trace: vec![],
            output,
            state,
        })
    }
    async fn call(
        &mut self,
        ctx: &ToolExecutionContext,
        name: &str,
        args: Value,
    ) -> anyhow::Result<ToolResult> {
        let result = self
            .registry
            .get(name)
            .unwrap()
            .execute(
                ctx.clone(),
                ToolCall {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: name.into(),
                    arguments: args.clone(),
                    raw_arguments: args.to_string(),
                    thread_id: ctx.thread_id.clone(),
                    turn_id: ctx.turn_id.clone(),
                },
            )
            .await?;
        let mut data = result.data.clone();
        let image = data.as_object_mut().unwrap().remove("__view_image");
        // Never persist full-desktop images or unrelated window inventories.
        if !matches!(
            name,
            "cua_get_desktop_state" | "cua_move_cursor" | "cua_list_windows" | "cua_list_apps"
        ) {
            if let Some(url) = image.as_ref().and_then(|i| i["image_url"].as_str()) {
                std::fs::write(
                    self.output.join(format!("{:02}.png", self.trace.len())),
                    base64::engine::general_purpose::STANDARD
                        .decode(url.split_once(',').unwrap().1)?,
                )?;
            }
            for key in ["observation", "after_action"] {
                redact_window(&mut data[key]);
            }
        } else {
            data = json!({"image_returned":image.is_some(),"inventory_or_desktop_content_omitted":true});
        }
        self.trace
            .push(json!({"tool":name,"arguments":args,"is_error":result.is_error,"data":data}));
        std::fs::write(
            self.output.join("trace.json"),
            serde_json::to_vec_pretty(&self.trace)?,
        )?;
        Ok(result)
    }
    async fn success(
        &mut self,
        ctx: &ToolExecutionContext,
        name: &str,
        args: Value,
    ) -> anyhow::Result<Value> {
        let result = self.call(ctx, name, args).await?;
        anyhow::ensure!(!result.is_error, "{name}: {}", result.text);
        Ok(result.data)
    }
    async fn observe(
        &mut self,
        ctx: &ToolExecutionContext,
        target: &Value,
    ) -> anyhow::Result<Value> {
        Ok(self
            .success(ctx, "cua_get_window_state", target.clone())
            .await?["observation"]
            .clone())
    }
    fn oracle(&self) -> anyhow::Result<Value> {
        Ok(serde_json::from_slice(&std::fs::read(&self.state)?)?)
    }
}
fn redact_window(value: &mut Value) {
    let mut keep = std::collections::HashSet::new();
    if let Some(rows) = value["elements"].as_array_mut() {
        rows.retain(|row| {
            let belongs = row["role"] == "AXWindow"
                || row["parent_index"]
                    .as_u64()
                    .is_some_and(|i| keep.contains(&i));
            if belongs && let Some(i) = row["element_index"].as_u64() {
                keep.insert(i);
            }
            belongs
        });
    }
    if let Some(tree) = value["tree_markdown"].as_str() {
        value["tree_markdown"] = json!(tree.split("\n- ").next().unwrap_or(tree));
    }
}
fn target(state: &Value, index: usize) -> Value {
    json!({"pid":state["pid"],"window_id":state["windows"][index]["number"]})
}
fn element<'a>(state: &'a Value, label: &str) -> anyhow::Result<&'a Value> {
    state["elements"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["label"] == label))
        .ok_or_else(|| anyhow::anyhow!("missing element {label}"))
}
fn input(target: &Value, extra: Value) -> Value {
    let mut args = target.clone();
    args.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    if args.get("value").is_none() {
        args["delivery_mode"] = json!("foreground");
    }
    args
}
fn center(state: &Value, label: &str) -> anyhow::Result<Value> {
    if label == "Pointer pad" {
        // App-owned fixture geometry converted from bottom-left AppKit points.
        let scale = state["screenshot_width"].as_f64().unwrap()
            / state["window_bounds"]["width"].as_f64().unwrap();
        return Ok(
            json!({"x":270.*scale,"y":(state["window_bounds"]["height"].as_f64().unwrap()-145.)*scale,"capture_id":state["capture_id"]}),
        );
    }
    let f = &element(state, label)?["screenshot_frame"];
    Ok(
        json!({"x":f["x"].as_f64().unwrap()+f["w"].as_f64().unwrap()/2.,"y":f["y"].as_f64().unwrap()+f["h"].as_f64().unwrap()/2.,"capture_id":state["capture_id"]}),
    )
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    anyhow::ensure!(cfg!(target_os = "macos"), "macOS only");
    let state = PathBuf::from(std::env::var("RODER_CUA_MACOS_STATE")?);
    let output = PathBuf::from(std::env::var("RODER_CUA_MACOS_OUTPUT")?);
    let mut h = Harness::new(output.clone(), state)?;
    let initial = h.oracle()?;
    let a = target(&initial, 0);
    let b = target(&initial, 1);
    let ctx = ToolExecutionContext::new("mac-input-a", "eval", PolicyMode::Bypass);
    let peer = ToolExecutionContext::new("mac-input-b", "eval", PolicyMode::Bypass);
    // Verify discovery, but persist only fixture identities in the report.
    let windows = h.success(&ctx, "cua_list_windows", json!({})).await?;
    anyhow::ensure!(
        windows["observation"]["windows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["pid"] == a["pid"] && w["window_id"] == a["window_id"]),
        "owned window not discoverable"
    );
    h.observe(&ctx, &a).await?;
    h.success(&ctx, "cua_bring_to_front", a.clone()).await?;
    let observed = h.observe(&ctx, &a).await?;
    let mut click = center(&observed, "Pointer pad")?;
    click["count"] = json!(2);
    h.success(&ctx, "cua_click", input(&a, click)).await?;
    let observed = h.observe(&ctx, &a).await?;
    let mut click = center(&observed, "Pointer pad")?;
    click["button"] = json!("right");
    h.success(&ctx, "cua_click", input(&a, click)).await?;
    let observed = h.observe(&ctx, &a).await?;
    let p = center(&observed, "Pointer pad")?;
    h.success(&ctx,"cua_drag",input(&a,json!({"from_x":p["x"],"from_y":p["y"],"to_x":p["x"].as_f64().unwrap()+50.,"to_y":p["y"].as_f64().unwrap()+15.,"capture_id":p["capture_id"],"duration_ms":350}))).await?;
    let observed = h.observe(&ctx, &a).await?;
    let mut p = center(&observed, "Pointer pad")?;
    p["direction"] = json!("down");
    p["amount"] = json!(3);
    h.success(&ctx, "cua_scroll", input(&a, p)).await?;
    let observed = h.observe(&ctx, &a).await?;
    h.success(
        &ctx,
        "cua_click",
        input(&a, center(&observed, "Primary input")?),
    )
    .await?;
    h.success(
        &ctx,
        "cua_type_text",
        input(&a, json!({"text":"café 日本語"})),
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(150)).await;
    anyhow::ensure!(
        h.oracle()?["fields"][0] == "café 日本語" && h.oracle()?["fields"][1] == "",
        "Unicode typing missed exact window"
    );
    let observed = h.observe(&ctx, &b).await?;
    h.success(&ctx,"cua_set_value",input(&b,json!({"element_token":element(&observed,"Sibling input")?["element_token"],"value":"sibling λ"}))).await?;
    h.observe(&ctx, &a).await?;
    h.success(
        &ctx,
        "cua_press_key",
        input(&a, json!({"key":"k","modifiers":["ctrl","shift"]})),
    )
    .await?;
    h.observe(&ctx, &a).await?;
    h.success(&ctx, "cua_bring_to_front", a.clone()).await?;
    h.observe(&ctx, &a).await?;
    let mut frame = a.clone();
    frame.as_object_mut().unwrap().extend(
        json!({"x":480,"y":180,"width":600,"height":420})
            .as_object()
            .unwrap()
            .clone(),
    );
    h.success(&ctx, "cua_set_window_frame", frame).await?;
    let observed = h.observe(&ctx, &a).await?;
    h.success(
        &ctx,
        "cua_click",
        input(
            &a,
            json!({"element_token":element(&observed,"Open dialog")?["element_token"]}),
        ),
    )
    .await?;
    let windows = h.success(&ctx, "cua_list_windows", json!({})).await?;
    let dialog = windows["observation"]["windows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["pid"] == a["pid"] && w["title"] == "Roder Cua Dialog")
        .ok_or_else(|| anyhow::anyhow!("dialog not discovered"))?;
    let dialog = json!({"pid":dialog["pid"],"window_id":dialog["window_id"]});
    let observed = h.observe(&ctx, &dialog).await?;
    h.success(&ctx,"cua_set_value",input(&dialog,json!({"element_token":element(&observed,"Dialog input")?["element_token"],"value":"Dialog 日本語 verified"}))).await?;
    let observed = h.observe(&ctx, &dialog).await?;
    let closed = h
        .call(
            &ctx,
            "cua_click",
            input(
                &dialog,
                json!({"element_token":element(&observed,"Accept")?["element_token"]}),
            ),
        )
        .await?;
    // Accept closes the addressed window. The original input succeeded, its
    // after-action window capture correctly fails, and the app oracle confirms
    // submission. Observe a remaining window; never replay the click.
    tokio::time::sleep(Duration::from_millis(150)).await;
    anyhow::ensure!(
        closed.is_error
            && h.oracle()?["accepted"] == "Dialog 日本語 verified"
            && h.oracle()?["dialog_open"] == false,
        "closed dialog was not independently confirmed"
    );
    let old = h.observe(&peer, &b).await?;
    let observed = h.observe(&ctx, &a).await?;
    h.success(
        &ctx,
        "cua_click",
        input(&a, center(&observed, "Pointer pad")?),
    )
    .await?;
    anyhow::ensure!(
        h.call(
            &peer,
            "cua_click",
            input(&b, json!({"x":1,"y":1,"capture_id":old["capture_id"]}))
        )
        .await?
        .is_error,
        "peer screenshot survived local input"
    );
    let observed = h.observe(&ctx, &a).await?;
    let p = center(&observed, "Pointer pad")?;
    let executor = h.registry.get("cua_drag").unwrap();
    let cancelled = ctx.clone();
    let task = tokio::spawn(async move {
        executor.execute(cancelled.clone(),ToolCall {
        id:"cancel-drag".into(),name:"cua_drag".into(),arguments:input(&a,json!({"from_x":p["x"],"from_y":p["y"],"to_x":p["x"].as_f64().unwrap()+60.,"to_y":p["y"].as_f64().unwrap()+15.,"capture_id":p["capture_id"],"duration_ms":2000})),raw_arguments:String::new(),thread_id:cancelled.thread_id,turn_id:cancelled.turn_id
    }).await
    });
    tokio::time::sleep(Duration::from_millis(700)).await;
    task.abort();
    let _ = task.await;
    let a = target(&initial, 0);
    anyhow::ensure!(
        h.call(&ctx, "cua_press_key", input(&a, json!({"key":"End"})))
            .await?
            .is_error,
        "cancelled input retained grounding"
    );
    h.observe(&ctx, &a).await?;
    let observed = h.observe(&ctx, &a).await?;
    let global_x = observed["window_bounds"]["x"].as_f64().unwrap() + 270.;
    let global_y = observed["window_bounds"]["y"].as_f64().unwrap()
        + observed["window_bounds"]["height"].as_f64().unwrap()
        - 145.;
    let desktop = h
        .success(
            &ctx,
            "cua_get_desktop_state",
            json!({"max_image_dimension":600}),
        )
        .await?["observation"]
        .clone();
    h.success(&ctx,"cua_move_cursor",json!({"x":global_x*desktop["screenshot_width"].as_f64().unwrap()/desktop["screen_width"].as_f64().unwrap(),"y":global_y*desktop["screenshot_height"].as_f64().unwrap()/desktop["screen_height"].as_f64().unwrap(),"capture_id":desktop["capture_id"]})).await?;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let cursor = h.oracle()?;
    let cursor_matches = (cursor["cursor_x"].as_f64().unwrap() - global_x).abs() < 2.
        && (cursor["cursor_y"].as_f64().unwrap() - global_y).abs() < 2.;
    #[cfg(unix)]
    let daemon_restart = if let Ok(owned_pid) = std::env::var("RODER_CUA_MACOS_OWNED_DAEMON_PID") {
        let owned_pid: u32 = owned_pid.parse()?;
        let config = CuaConfig {
            backend: CuaBackend::LocalMacos,
            ..Default::default()
        };
        let socket = config.local_socket_path()?;
        let previous = std::fs::metadata(&socket)?;
        use std::os::unix::fs::MetadataExt;
        let observed = h.observe(&ctx, &a).await?;
        let stop = tokio::process::Command::new(config.program())
            .args([
                "stop",
                "--expected-pid",
                &owned_pid.to_string(),
                "--socket",
                &socket,
            ])
            .output()
            .await?;
        anyhow::ensure!(stop.status.success(), "owned daemon stop was refused");
        let launched = tokio::process::Command::new("/usr/bin/open")
            .args([
                "-n",
                "-g",
                "--env",
                "CUA_DRIVER_RS_TELEMETRY_ENABLED=0",
                "/Applications/CuaDriver.app",
                "--args",
                "serve",
            ])
            .status()
            .await?;
        anyhow::ensure!(launched.success(), "signed app launch failed");
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if let Ok(current) = std::fs::metadata(&socket)
                    && (current.ino(), current.ctime(), current.ctime_nsec())
                        != (previous.ino(), previous.ctime(), previous.ctime_nsec())
                    && std::os::unix::net::UnixStream::connect(&socket).is_ok()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await?;
        let refused = h
            .call(
                &ctx,
                "cua_click",
                input(&a, center(&observed, "Pointer pad")?),
            )
            .await?;
        anyhow::ensure!(
            refused.is_error && refused.data.get("__view_image").is_some(),
            "daemon restart did not reject stale input and recover capture"
        );
        h.observe(&ctx, &a).await?;
        true
    } else {
        false
    };
    #[cfg(not(unix))]
    let daemon_restart = false;
    // A new contributor is a fresh local session; it cannot reuse old grounding.
    let mut restarted = ToolRegistry::default();
    CuaToolContributor::new(CuaConfig {
        backend: CuaBackend::LocalMacos,
        ..Default::default()
    })
    .contribute(&mut restarted)?;
    let result = restarted
        .get("cua_press_key")
        .unwrap()
        .execute(
            ctx.clone(),
            ToolCall {
                id: "fresh-runtime".into(),
                name: "cua_press_key".into(),
                arguments: input(&a, json!({"key":"End"})),
                raw_arguments: String::new(),
                thread_id: ctx.thread_id.clone(),
                turn_id: ctx.turn_id.clone(),
            },
        )
        .await?;
    anyhow::ensure!(result.is_error, "runtime retained grounding");
    tokio::time::sleep(Duration::from_millis(150)).await;
    let state = h.oracle()?;
    let mut checks = json!({"discovery":true,"desktop_capture_and_scaled_cursor":cursor_matches,"double_click":state["doubles"].as_u64().unwrap_or(0)>0,"right_click":state["rights"].as_u64().unwrap_or(0)>0,"drag":state["drags"].as_u64().unwrap_or(0)>0,"scroll":state["scrolls"].as_u64().unwrap_or(0)>0,"unicode_typing":state["fields"][0]=="café 日本語","exact_sibling_value":state["fields"][1]=="sibling λ","dialog":state["accepted"]=="Dialog 日本語 verified","closed_window_capture_error":true,"no_held_button":state["held"]==false,"chord":state["keys"].as_array().unwrap().iter().any(|key|key["control"]==true && key["shift"]==true && key["window"]=="Roder Cua Input A"),"resize":(state["windows"][0]["width"].as_f64().unwrap()-600.).abs()<=2. && (state["windows"][0]["height"].as_f64().unwrap()-420.).abs()<=2.,"cancellation":true,"cross_thread_generation":true,"runtime_restart_requires_observation":true});
    if daemon_restart {
        checks["daemon_restart"] = json!(true);
    }
    let passed = checks
        .as_object()
        .unwrap()
        .values()
        .all(|value| value == true);
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(
            &json!({"passed":passed,"checks":checks,"app_oracle":state,"backend":"local-macos","driver_version":"0.34.0","live_model":false,"daemon_restart_exercised":daemon_restart}),
        )?,
    )?;
    println!("{}", json!({"passed":passed,"checks":checks}));
    anyhow::ensure!(passed, "native macOS input qualification failed");
    Ok(())
}
