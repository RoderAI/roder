//! Opt-in live qualification against two explicitly owned, disposable desktops.
//! Provision examples/cua-linux/input_fixture.py first. Never creates a sandbox.
use base64::Engine;
use roder_api::{policy_mode::PolicyMode, remote_runner::*, tools::*};
use roder_ext_cua::{CuaConfig, CuaToolContributor};
use roder_ext_runner_blaxel::BlaxelRunnerProvider;
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::Duration};

struct Harness {
    registry: ToolRegistry,
    trace: Vec<Value>,
    output: PathBuf,
}
impl Harness {
    fn new(output: PathBuf) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&output)?;
        let mut registry = ToolRegistry::default();
        CuaToolContributor::new(CuaConfig {
            max_image_dimension: 280,
            ..Default::default()
        })
        .contribute(&mut registry)?;
        Ok(Self {
            registry,
            trace: vec![],
            output,
        })
    }
    async fn call(
        &mut self,
        ctx: &ToolExecutionContext,
        name: &str,
        args: Value,
    ) -> anyhow::Result<ToolResult> {
        let remote = ctx.handles.remote_workspace.as_ref().unwrap();
        let lease = remote
            .session
            .acquire_workspace_execution_lease(RunnerWorkspaceExecutionLeaseRequest {
                execution_id: uuid::Uuid::new_v4().to_string(),
                acquire_timeout_ms: 45000,
                lease_timeout_ms: Some(95000),
            })
            .await?;
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
        if let Some(lease) = lease {
            lease.release().await?;
        }
        if let Some(url) = result.data["__view_image"]["image_url"].as_str() {
            let data =
                base64::engine::general_purpose::STANDARD.decode(url.split_once(',').unwrap().1)?;
            std::fs::write(
                self.output.join(format!("{:02}.png", self.trace.len())),
                data,
            )?;
        }
        self.trace.push(
            json!({"tool":name,"arguments":args,"is_error":result.is_error,"output":result.text}),
        );
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
}
fn target(window: &Value) -> Value {
    json!({"pid":window["pid"],"window_id":window["window_id"]})
}
fn window<'a>(windows: &'a Value, title: &str) -> anyhow::Result<&'a Value> {
    windows["observation"]["windows"]
        .as_array()
        .and_then(|windows| {
            windows.iter().find(|w| {
                w["title"]
                    .as_str()
                    .or_else(|| w["window_title"].as_str())
                    .is_some_and(|t| t == title)
            })
        })
        .ok_or_else(|| anyhow::anyhow!("window {title} not found in {windows}"))
}
fn element(observation: &Value, label: &str) -> anyhow::Result<String> {
    observation["elements"]
        .as_array()
        .and_then(|elements| elements.iter().find(|e| e["label"] == label))
        .and_then(|e| e["element_token"].as_str())
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("element {label} not found in {observation}"))
}
async fn oracle(ctx: &ToolExecutionContext) -> anyhow::Result<Value> {
    let file = ctx
        .handles
        .remote_workspace
        .as_ref()
        .unwrap()
        .session
        .read_file(RunnerFileReadRequest {
            path: "/tmp/roder-cua-input-oracle.json".into(),
        })
        .await?;
    Ok(serde_json::from_slice(&file.contents)?)
}
fn pixel(observation: &Value, state: &Value, name: &str, dx: f64, dy: f64) -> Value {
    let rect = &state["targets"][name];
    let scale = observation["screenshot_width"].as_f64().unwrap()
        / state["geometry"]["width"].as_f64().unwrap();
    json!({"x":(rect["x"].as_f64().unwrap()+dx)*scale,"y":(rect["y"].as_f64().unwrap()+dy)*scale,"capture_id":observation["capture_id"]})
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
async fn connect(
    provider: &BlaxelRunnerProvider,
    name: String,
    workspace: &str,
) -> anyhow::Result<ToolExecutionContext> {
    let session=provider.create_session(RunnerDestination {
        id:name.clone(),provider_id:"blaxel".into(),default_manifest:Default::default(),
        config:json!({"sandbox_name":name,"workspace":workspace,"image":"blaxel/cua-xfce:latest","cleanup":"detach-on-close"})
    }).await?;
    Ok(
        ToolExecutionContext::new(name, "input-eval", PolicyMode::Bypass).with_remote_workspace(
            Arc::new(RemoteWorkspace {
                session,
                root: "/home/cua".into(),
                read_roots: vec![],
            }),
        ),
    )
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let names = std::env::var("RODER_CUA_INPUT_SANDBOXES")?;
    let names = names.split(',').collect::<Vec<_>>();
    anyhow::ensure!(
        names.len() == 2 && names[0] != names[1],
        "two distinct owned sandboxes required"
    );
    let workspace = std::env::var("RODER_CUA_INPUT_WORKSPACE")?;
    let output = PathBuf::from(std::env::var("RODER_CUA_INPUT_OUTPUT")?);
    let provider = BlaxelRunnerProvider::default();
    let a = connect(&provider, names[0].into(), &workspace).await?;
    let b = connect(&provider, names[1].into(), &workspace).await?;
    let mut h = Harness::new(output.clone())?;
    let windows = h.success(&a, "cua_list_windows", json!({})).await?;
    let main_target = target(window(&windows, "Roder Cua Input Fixture")?);
    let initial = h.observe(&a, &main_target).await?;
    let other_windows = h.success(&b, "cua_list_windows", json!({})).await?;
    let other_target = target(window(&other_windows, "Roder Cua Input Fixture")?);
    h.observe(&b, &other_target).await?;
    let mut foreign = other_target.clone();
    foreign.as_object_mut().unwrap().extend(
        pixel(&initial, &oracle(&a).await?, "pointer", 40., 40.)
            .as_object()
            .unwrap()
            .clone(),
    );
    anyhow::ensure!(
        h.call(&b, "cua_click", foreign).await?.is_error,
        "foreign capture accepted"
    );
    anyhow::ensure!(h.call(&b,"cua_set_value",input(&other_target,json!({"value":"foreign token forbidden","element_token":element(&initial,"Unicode input")?}))).await?.is_error,"foreign element token accepted");
    let observed = h.observe(&a, &main_target).await?;
    let mut click = pixel(&observed, &oracle(&a).await?, "pointer", 40., 40.);
    click["count"] = json!(2);
    h.success(&a, "cua_click", input(&main_target, click))
        .await?;
    let observed = h.observe(&a, &main_target).await?;
    let mut click = pixel(&observed, &oracle(&a).await?, "pointer", 40., 40.);
    click["button"] = json!("right");
    h.success(&a, "cua_click", input(&main_target, click))
        .await?;
    let observed = h.observe(&a, &main_target).await?;
    let state = oracle(&a).await?;
    let from = pixel(&observed, &state, "pointer", 40., 40.);
    let to = pixel(&observed, &state, "pointer", 240., 90.);
    h.success(&a,"cua_drag",input(&main_target,json!({"from_x":from["x"],"from_y":from["y"],"to_x":to["x"],"to_y":to["y"],"capture_id":observed["capture_id"],"duration_ms":500}))).await?;
    let observed = h.observe(&a, &main_target).await?;
    let mut scroll = pixel(&observed, &oracle(&a).await?, "scroll", 100., 40.);
    scroll["direction"] = json!("down");
    scroll["amount"] = json!(3);
    h.success(&a, "cua_scroll", input(&main_target, scroll))
        .await?;
    let observed = h.observe(&a, &main_target).await?;
    h.success(
        &a,
        "cua_click",
        input(
            &main_target,
            json!({"element_token":element(&observed,"Unicode input")?}),
        ),
    )
    .await?;
    let observed = h.observe(&a, &main_target).await?;
    h.success(&a,"cua_set_value",input(&main_target,json!({"value":"Roder café λ 日本語","element_token":element(&observed,"Unicode input")?}))).await?;
    h.observe(&a, &main_target).await?;
    h.success(
        &a,
        "cua_press_key",
        input(&main_target, json!({"key":"k","modifiers":["ctrl"]})),
    )
    .await?;
    h.observe(&a, &main_target).await?;
    h.success(&a, "cua_bring_to_front", main_target.clone())
        .await?;
    h.observe(&a, &main_target).await?;
    h.success(&a, "cua_set_window_frame", {
        let mut frame = main_target.clone();
        frame.as_object_mut().unwrap().extend(
            json!({"x":90,"y":90,"width":620,"height":480})
                .as_object()
                .unwrap()
                .clone(),
        );
        frame
    })
    .await?;
    let observed = h.observe(&a, &main_target).await?;
    let button = element(&observed, "Open dialog")?;
    let state = oracle(&a).await?;
    let desktop = h
        .success(
            &a,
            "cua_get_desktop_state",
            json!({"max_image_dimension":0}),
        )
        .await?["observation"]
        .clone();
    h.success(&a,"cua_move_cursor",json!({"x":observed["window_bounds"]["x"].as_f64().unwrap()+state["targets"]["pointer"]["x"].as_f64().unwrap()+60.,"y":observed["window_bounds"]["y"].as_f64().unwrap()+state["targets"]["pointer"]["y"].as_f64().unwrap()+60.,"capture_id":desktop["capture_id"]})).await?;
    // The desktop observation replaced window grounding; observe before using its token.
    let observed = h.observe(&a, &main_target).await?;
    let _ = button;
    h.success(
        &a,
        "cua_click",
        input(
            &main_target,
            json!({"element_token":element(&observed,"Open dialog")?}),
        ),
    )
    .await?;
    let windows = h.success(&a, "cua_list_windows", json!({})).await?;
    let dialog_target = target(window(&windows, "Roder Cua Dialog")?);
    let observed = h.observe(&a, &dialog_target).await?;
    h.success(
        &a,
        "cua_set_value",
        input(
            &dialog_target,
            json!({"value":"Dialog verified","element_token":element(&observed,"Dialog input")?}),
        ),
    )
    .await?;
    let session = &a.handles.remote_workspace.as_ref().unwrap().session;
    session.pause().await?;
    session.resume().await?;
    let persisted = session.detach().await?;
    let rejoined = provider.rejoin_session(persisted).await?;
    let rectx = ToolExecutionContext::new(a.thread_id.clone(), "rejoin", PolicyMode::Bypass)
        .with_remote_workspace(Arc::new(RemoteWorkspace {
            session: rejoined.clone(),
            root: "/home/cua".into(),
            read_roots: vec![],
        }));
    let mut joined = Harness::new(output.join("rejoin"))?;
    anyhow::ensure!(
        joined
            .call(
                &rectx,
                "cua_press_key",
                input(&dialog_target, json!({"key":"END"}))
            )
            .await?
            .is_error,
        "rejoin retained old grounding"
    );
    let observed = joined.observe(&rectx, &dialog_target).await?;
    rejoined
        .run_command(RunnerCommandRequest {
            command_id: uuid::Uuid::new_v4().to_string(),
            program: "/usr/bin/pkill".into(),
            args: vec!["-x".into(), "cua-driver".into()],
            cwd: None,
            env: vec![],
            timeout_ms: Some(5000),
        })
        .await?;
    let stale = joined
        .call(
            &rectx,
            "cua_click",
            input(
                &dialog_target,
                json!({"x":10,"y":10,"capture_id":observed["capture_id"]}),
            ),
        )
        .await?;
    anyhow::ensure!(
        stale.is_error && stale.data.get("__view_image").is_some(),
        "restart did not refuse stale input and recover capture"
    );
    joined.observe(&rectx, &main_target).await?;
    let mut cancel_args = main_target.clone();
    let observed = joined.observe(&rectx, &main_target).await?;
    let state = oracle(&rectx).await?;
    let from = pixel(&observed, &state, "pointer", 40., 40.);
    let to = pixel(&observed, &state, "pointer", 240., 90.);
    cancel_args.as_object_mut().unwrap().extend(json!({"from_x":from["x"],"from_y":from["y"],"to_x":to["x"],"to_y":to["y"],"capture_id":observed["capture_id"],"duration_ms":5000,"delivery_mode":"foreground"}).as_object().unwrap().clone());
    let executor = joined.registry.get("cua_drag").unwrap();
    let ctx = rectx.clone();
    let task = tokio::spawn(async move {
        executor
            .execute(
                ctx.clone(),
                ToolCall {
                    id: "cancel-drag".into(),
                    name: "cua_drag".into(),
                    arguments: cancel_args,
                    raw_arguments: String::new(),
                    thread_id: ctx.thread_id,
                    turn_id: ctx.turn_id,
                },
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(700)).await;
    task.abort();
    let _ = task.await;
    anyhow::ensure!(
        joined
            .call(
                &rectx,
                "cua_press_key",
                input(&main_target, json!({"key":"END"}))
            )
            .await?
            .is_error,
        "cancelled action retained grounding"
    );
    joined.observe(&rectx, &main_target).await?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let state = oracle(&rectx).await?;
    let other = oracle(&b).await?;
    let checks = json!({"double_click":state["double_clicks"].as_u64().unwrap_or(0)>0,"right_click":state["right_clicks"].as_u64().unwrap_or(0)>0,
        "drag":state["drags"].as_u64().unwrap_or(0)>0,"scroll":state["scrolls"].as_u64().unwrap_or(0)>0,"unicode":state["text"]=="Roder café λ 日本語",
        "modifier":state["chords"].as_u64().unwrap_or(0)>0,"dialog":state["dialog_text"]=="Dialog verified","cursor":state["motions"].as_u64().unwrap_or(0)>0,
        "focus":state["focus_seen"]==true,"resize":state["geometry"]["width"]==620 && state["geometry"]["height"]==480,"no_held_button":state["held_button"]==false,
        "isolation":other["text"]=="" && other["buttons"].as_array().is_some_and(Vec::is_empty),"pause_resume":true,"rejoin":true,"driver_restart":true,"cancellation":true});
    let passed = checks.as_object().unwrap().values().all(|v| v == true);
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(
            &json!({"passed":passed,"checks":checks,"app_oracle":state,"other_app_oracle":other,"sandboxes":names,"backend":"XFCE/X11","driver_version":"0.34.0","live_model":false}),
        )?,
    )?;
    rejoined.close().await?;
    session.close().await?;
    b.handles
        .remote_workspace
        .as_ref()
        .unwrap()
        .session
        .close()
        .await?;
    println!("{}", json!({"passed":passed,"checks":checks}));
    anyhow::ensure!(passed, "input qualification failed");
    Ok(())
}
