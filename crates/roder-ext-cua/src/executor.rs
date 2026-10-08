use crate::capture::{Capture, take_capture};
use crate::specs::{INPUT_TOOLS, READ_TOOLS, is_input, spec};
use crate::{CuaConfig, CuaTransport, DriverReply, RunnerCuaTransport};
use async_trait::async_trait;
use roder_api::policy_mode::PolicyMode;
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult,
    ToolSpec,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

struct Grounding {
    target: Value,
    capture: Capture,
}
#[derive(Default)]
struct Session {
    grounding: Option<Grounding>,
}
struct Shared {
    config: CuaConfig,
    transport: Arc<dyn CuaTransport>,
    sessions: Mutex<HashMap<String, Arc<Mutex<Session>>>>,
}
pub struct CuaToolContributor {
    shared: Arc<Shared>,
}
impl CuaToolContributor {
    pub fn new(config: CuaConfig) -> Self {
        Self::with_transport(config.clone(), Arc::new(RunnerCuaTransport::new(&config)))
    }
    pub fn with_transport(config: CuaConfig, transport: Arc<dyn CuaTransport>) -> Self {
        Self {
            shared: Arc::new(Shared {
                config,
                transport,
                sessions: Mutex::new(HashMap::new()),
            }),
        }
    }
}
impl ToolContributor for CuaToolContributor {
    fn id(&self) -> String {
        "cua".into()
    }
    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        self.shared.config.validate()?;
        for name in READ_TOOLS.iter().chain(INPUT_TOOLS) {
            registry.register(Arc::new(CuaTool {
                name,
                shared: self.shared.clone(),
            }))?;
        }
        Ok(())
    }
}
struct CuaTool {
    name: &'static str,
    shared: Arc<Shared>,
}
#[async_trait]
impl ToolExecutor for CuaTool {
    fn spec(&self) -> ToolSpec {
        spec(self.name)
    }
    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let result = self.run(&ctx, &call).await;
        Ok(match result {
            Ok(result) => result,
            Err(error) => ToolResult {
                id: call.id,
                name: call.name,
                text: error.to_string(),
                data: json!({"untrusted":true}),
                is_error: true,
            },
        })
    }
}
impl CuaTool {
    async fn run(&self, ctx: &ToolExecutionContext, call: &ToolCall) -> anyhow::Result<ToolResult> {
        anyhow::ensure!(
            call.thread_id == ctx.thread_id
                && call.turn_id == ctx.turn_id
                && call.name == self.spec().name,
            "Cua tool context does not match the call"
        );
        let mut arguments = crate::specs::arguments(self.name, &call.arguments)?;
        anyhow::ensure!(
            !(is_input(self.name) && ctx.effective_mode == PolicyMode::Plan),
            "desktop input is disabled in Plan mode"
        );
        let remote = ctx.handles.remote_workspace.as_ref().ok_or_else(|| anyhow::anyhow!("Cua requires a thread bound to a graphical remote runner. Local desktop fallback is disabled."))?;
        let state = remote.session.state();
        let label = session_label(&ctx.thread_id, &state.provider_id, &state.session_id);
        let session = {
            let mut sessions = self.shared.sessions.lock().await;
            if sessions.len() >= 256 && !sessions.contains_key(&label) {
                // Eviction only invalidates grounding; input then requires a fresh read.
                let idle = sessions
                    .iter()
                    .find(|(_, session)| Arc::strong_count(session) == 1)
                    .map(|(key, _)| key.clone());
                if let Some(idle) = idle {
                    sessions.remove(&idle);
                }
                anyhow::ensure!(sessions.len() < 256, "too many concurrent Cua sessions");
            }
            sessions.entry(label.clone()).or_default().clone()
        };
        let mut session = session.lock().await;
        let timeout = self.shared.config.timeout_ms.min(
            ctx.deadline_remaining_seconds
                .map(|s| s.saturating_mul(1000))
                .unwrap_or(u64::MAX),
        );
        anyhow::ensure!(timeout >= 1000, "insufficient turn time for desktop action");
        let target = if self.name == "move_cursor" || self.name == "get_desktop_state" {
            json!({"desktop":true})
        } else {
            json!({"pid":arguments["pid"],"window_id":arguments["window_id"]})
        };
        if is_input(self.name) {
            check_grounding(&session, &arguments, &target)?;
            // Discard before dispatch: cancellation, timeout or a failed capture must
            // never leave an old image authorized for another action.
            session.grounding = None;
        }
        if matches!(self.name, "get_window_state" | "get_desktop_state") {
            session.grounding = None;
        }
        if matches!(self.name, "drag" | "scroll" | "move_cursor") {
            arguments.as_object_mut().unwrap().remove("capture_id");
        }
        if self.name == "move_cursor" {
            arguments["scope"] = json!("desktop");
        }
        arguments["session"] = json!(label);
        if let Some(token) = arguments["element_token"].as_str() {
            // Grounding above checked the bound snapshot. Only the driver sees
            // its original token, never another runner's opaque public handle.
            arguments["element_token"] = json!(
                token
                    .split_once('/')
                    .ok_or_else(|| anyhow::anyhow!("invalid bound element token"))?
                    .1
            );
        }
        if matches!(self.name, "get_window_state" | "get_desktop_state")
            && arguments.get("max_image_dimension").is_none()
        {
            arguments["max_image_dimension"] = json!(self.shared.config.max_image_dimension);
        }
        let mut data = json!({"untrusted":true,"driver_version":crate::DRIVER_VERSION});
        let mut failed = false;
        let mut messages = vec![];
        if self.name == "type_text" {
            // The pinned driver's generic AT-SPI typing can choose a sibling
            // editable. Its set_value route preserves exact window identity.
            let windows = self
                .shared
                .transport
                .call(remote, "list_windows", json!({"session":label}), timeout)
                .await?;
            let windows = windows.observation["windows"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("cannot verify typing target"))?;
            anyhow::ensure!(
                windows
                    .iter()
                    .filter(|window| window["pid"] == target["pid"])
                    .count()
                    == 1
                    && windows.iter().any(|window| window["pid"] == target["pid"]
                        && window["window_id"] == target["window_id"]),
                "Cua 0.34 typing requires a single-window application; use cua_set_value for an exact editable"
            );
        }
        match self
            .shared
            .transport
            .call(remote, self.name, arguments, timeout)
            .await
        {
            Ok(mut reply) => {
                failed |= reply.is_error;
                if !is_input(self.name)
                    && matches!(self.name, "get_window_state" | "get_desktop_state")
                    && !reply.is_error
                {
                    let mut capture = take_capture(&mut reply.observation, &label)?;
                    data[roder_api::transcript::VIEW_IMAGE_DISPLAY_KEY] = capture.image.take();
                    session.grounding = Some(Grounding {
                        target: target.clone(),
                        capture,
                    });
                }
                data["observation"] = reply.observation;
            }
            Err(error) => {
                failed = true;
                messages.push(error.to_string());
            }
        }
        if is_input(self.name) {
            let tool = if self.name == "move_cursor" {
                "get_desktop_state"
            } else {
                "get_window_state"
            };
            let mut capture_args = if tool == "get_window_state" {
                target.clone()
            } else {
                json!({})
            };
            capture_args["session"] = json!(label);
            capture_args["max_image_dimension"] = json!(self.shared.config.max_image_dimension);
            match self
                .shared
                .transport
                .call(remote, tool, capture_args, timeout)
                .await
            {
                Ok(DriverReply {
                    mut observation,
                    is_error: false,
                }) => match take_capture(&mut observation, &label) {
                    Ok(mut capture) => {
                        data[roder_api::transcript::VIEW_IMAGE_DISPLAY_KEY] = capture.image.take();
                        session.grounding = Some(Grounding { target, capture });
                        data["after_action"] = observation;
                    }
                    Err(error) => {
                        failed = true;
                        messages.push(format!(
                            "After-action capture failed: {error}. Observe again before input."
                        ));
                    }
                },
                _ => {
                    failed = true;
                    messages.push("After-action capture failed. Observe again before input; no action was retried.".into());
                }
            }
        }
        if !messages.is_empty() {
            data["errors"] = json!(messages);
        }
        let mut text_data = data.clone();
        text_data
            .as_object_mut()
            .unwrap()
            .remove(roder_api::transcript::VIEW_IMAGE_DISPLAY_KEY);
        Ok(ToolResult {
            id: call.id.clone(),
            name: call.name.clone(),
            text: text_data.to_string(),
            data,
            is_error: failed,
        })
    }
}
fn session_label(thread: &str, provider: &str, runner: &str) -> String {
    let hash = Sha256::digest(serde_json::to_vec(&(thread, provider, runner)).unwrap());
    format!(
        "roder-{}",
        hash.iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}
fn check_grounding(session: &Session, args: &Value, target: &Value) -> anyhow::Result<()> {
    let ground = session.grounding.as_ref().ok_or_else(|| {
        anyhow::anyhow!("observe this target before using pixel or element input")
    })?;
    anyhow::ensure!(
        &ground.target == target,
        "grounding belongs to another window or desktop"
    );
    if let Some(token) = args["element_token"].as_str() {
        let (snapshot, row) = token
            .rsplit_once(':')
            .ok_or_else(|| anyhow::anyhow!("invalid element_token"))?;
        anyhow::ensure!(
            ground.capture.snapshot.as_deref() == Some(snapshot)
                && !row.is_empty()
                && row.bytes().all(|b| b.is_ascii_digit()),
            "stale or foreign element_token; observe again"
        );
    }
    if let Some(id) = args["capture_id"].as_str() {
        anyhow::ensure!(
            id == ground.capture.id,
            "stale or foreign capture_id; observe again"
        );
        for (x, y) in [("x", "y"), ("from_x", "from_y"), ("to_x", "to_y")] {
            if let Some(px) = args[x].as_f64() {
                let py = args[y]
                    .as_f64()
                    .ok_or_else(|| anyhow::anyhow!("missing {y}"))?;
                anyhow::ensure!(
                    px < f64::from(ground.capture.width) && py < f64::from(ground.capture.height),
                    "coordinates exceed the grounding PNG"
                );
            }
        }
    }
    Ok(())
}
