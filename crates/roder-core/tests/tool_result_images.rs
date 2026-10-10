//! Tool-result screenshots versus engines that cannot receive them (report item 7).
//!
//! A scripted engine drives a fake browser tool for 30 rounds. Only engines
//! that return `tool_result_image_input() == true` may see the image payload;
//! for the others the request copy drops it, says so once per result, and the
//! screenshot-only tools are not advertised. The persisted transcript is never
//! rewritten, so a mid-thread switch between the two kinds of engine replays
//! the same history.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::stream;
use roder_api::events::RoderEvent;
use roder_api::extension::{ExtensionRegistryBuilder, InferenceEngineId, ToolProviderId};
use roder_api::inference::*;
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolResult, ToolSpec,
};
use roder_api::transcript::TranscriptItem;
use roder_core::{Runtime, RuntimeConfig, StartTurnRequest, default_instructions};
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
use serde_json::json;

const NOTICE: &str = "screenshot not shown: this model cannot receive images in tool results";
const IMAGE_URL: &str = "data:image/png;base64,YWJj";
const ACTION_TEXT: &str = "Clicked element 3. Screenshot attached.";
const SCREENSHOT_ONLY_TOOLS: [&str; 4] = [
    "browser_use_screenshot",
    "chrome_screenshot",
    "jev_tab_screenshot",
    "view_image",
];

/// Drives `steps` tool rounds per user message (calling `tool`), then answers.
/// `forwards_images_for` is the model, if any, whose tool-result images it takes.
struct ScriptedEngine {
    id: &'static str,
    tool: &'static str,
    forwards_images_for: Option<&'static str>,
    steps: AtomicUsize,
    requests: Mutex<Vec<AgentInferenceRequest>>,
}

impl ScriptedEngine {
    fn new(id: &'static str, steps: usize) -> Arc<Self> {
        Arc::new(Self {
            id,
            tool: "browser_use_click",
            forwards_images_for: None,
            steps: AtomicUsize::new(steps),
            requests: Mutex::new(Vec::new()),
        })
    }

    /// An engine that forwards tool-result images for every model.
    fn seeing(id: &'static str, tool: &'static str, steps: usize) -> Arc<Self> {
        Arc::new(Self {
            id,
            tool,
            forwards_images_for: Some("scripted-model"),
            steps: AtomicUsize::new(steps),
            requests: Mutex::new(Vec::new()),
        })
    }

    fn requests(&self) -> Vec<AgentInferenceRequest> {
        self.requests.lock().unwrap().clone()
    }

    fn set_steps(&self, steps: usize) {
        self.steps.store(steps, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl InferenceEngine for ScriptedEngine {
    fn id(&self) -> InferenceEngineId {
        self.id.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::coding_agent_default()
    }

    fn tool_result_image_input(&self, model: &str) -> bool {
        self.forwards_images_for == Some(model)
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(Vec::new())
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let rounds_this_turn = request
            .transcript
            .iter()
            .rev()
            .take_while(|item| !matches!(item, TranscriptItem::UserMessage(_)))
            .filter(|item| matches!(item, TranscriptItem::ToolResult(_)))
            .count();
        let call_id = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request);
            format!("call_{}_{}", self.id, requests.len())
        };
        let events = if rounds_this_turn < self.steps.load(Ordering::SeqCst) {
            vec![
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: call_id,
                    name: self.tool.to_string(),
                    arguments: "{}".to_string(),
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("tool_calls".to_string()),
                    provider_response_id: None,
                })),
            ]
        } else {
            vec![
                Ok(InferenceEvent::MessageDelta(MessageDelta {
                    text: "done".to_string(),
                    phase: None,
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("stop".to_string()),
                    provider_response_id: None,
                })),
            ]
        };
        Ok(Box::pin(stream::iter(events)))
    }
}

struct FakeBrowserContributor;

impl ToolContributor for FakeBrowserContributor {
    fn id(&self) -> ToolProviderId {
        "test-fake-browser".to_string()
    }

    fn contribute(&self, registry: &mut roder_api::tools::ToolRegistry) -> anyhow::Result<()> {
        for name in ["browser_use_click"]
            .into_iter()
            .chain(SCREENSHOT_ONLY_TOOLS)
        {
            registry.register(Arc::new(FakeBrowserTool { name }))?;
        }
        Ok(())
    }
}

/// Every tool returns text that says a screenshot is attached, plus the image.
struct FakeBrowserTool {
    name: &'static str,
}

#[async_trait::async_trait]
impl ToolExecutor for FakeBrowserTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name.to_string(),
            description: "Fake browser tool that returns a screenshot".to_string(),
            parameters: json!({ "type": "object" }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: ACTION_TEXT.to_string(),
            data: json!({ "__view_image": { "image_url": IMAGE_URL, "detail": "original" } }),
            is_error: false,
        })
    }
}

struct Harness {
    runtime: Arc<Runtime>,
    events: tokio::sync::broadcast::Receiver<roder_api::events::EventEnvelope>,
    thread: String,
    _directory: tempfile::TempDir,
}

fn harness(engines: &[Arc<ScriptedEngine>]) -> Harness {
    let directory = tempfile::tempdir().unwrap();
    let mut builder = ExtensionRegistryBuilder::new();
    for engine in engines {
        builder.inference_engine(engine.clone());
    }
    builder.tool_contributor(Arc::new(FakeBrowserContributor));
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: directory.path().to_path_buf(),
    }));
    let runtime =
        Arc::new(Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap());
    let events = runtime.subscribe_events();
    Harness {
        runtime,
        events,
        thread: "thread_tool_result_images".to_string(),
        _directory: directory,
    }
}

impl Harness {
    /// Runs one user message on `provider` and returns the prompt token
    /// estimate that was recorded when its context was assembled.
    async fn run(&mut self, provider: &str, model: &str, message: &str) -> u32 {
        self.runtime
            .start_turn(StartTurnRequest {
                thread_id: self.thread.clone(),
                message: message.to_string(),
                images: Vec::new(),
                provider_override: Some(provider.to_string()),
                model_override: Some(model.to_string()),
                reasoning_override: None,
                workspace: std::env::current_dir().unwrap().display().to_string(),
                instructions: default_instructions(),
                developer_context: None,
                task_ledger_required: false,
                service_tier_override: None,
            })
            .await
            .unwrap();
        let mut assembled = None;
        loop {
            let event = tokio::time::timeout(Duration::from_secs(30), self.events.recv())
                .await
                .expect("turn did not finish")
                .unwrap();
            if event.thread_id.as_deref() != Some(self.thread.as_str()) {
                continue;
            }
            assert_ne!(event.kind, "turn.failed", "{event:?}");
            if let RoderEvent::ContextAssemblyCompleted(done) = &event.event {
                assembled.get_or_insert(done.prompt_estimated_tokens);
            }
            if event.kind == "turn.completed" {
                return assembled.expect("context assembly is recorded for every turn");
            }
        }
    }

    async fn persisted_tool_results(&self) -> Vec<roder_api::transcript::ToolResultRecord> {
        let snapshot = self
            .runtime
            .load_thread(&self.thread)
            .await
            .unwrap()
            .unwrap();
        snapshot
            .turns
            .into_iter()
            .flat_map(|turn| turn.items)
            .filter_map(|item| match item {
                TranscriptItem::ToolResult(result) => Some(result),
                _ => None,
            })
            .collect()
    }
}

fn tool_results(request: &AgentInferenceRequest) -> Vec<&roder_api::transcript::ToolResultRecord> {
    request
        .transcript
        .iter()
        .filter_map(|item| match item {
            TranscriptItem::ToolResult(result) => Some(result),
            _ => None,
        })
        .collect()
}

fn tool_names(request: &AgentInferenceRequest) -> Vec<&str> {
    request
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect()
}

async fn blind_run() -> (
    Vec<AgentInferenceRequest>,
    Vec<roder_api::transcript::ToolResultRecord>,
) {
    let engine = ScriptedEngine::new("mock-blind", 30);
    let mut harness = harness(std::slice::from_ref(&engine));
    harness
        .run("mock-blind", "scripted-model", "click through the page")
        .await;
    (engine.requests(), harness.persisted_tool_results().await)
}

#[tokio::test]
async fn text_only_engine_gets_no_image_payload_and_one_notice_per_result() {
    let (requests, _) = blind_run().await;
    assert_eq!(requests.len(), 31, "30 tool rounds and one answer");
    for (round, request) in requests.iter().enumerate() {
        let wire = serde_json::to_string(&request.transcript).unwrap();
        assert!(
            !wire.contains("data:image"),
            "round {round} carries a data URL"
        );
        assert!(
            !wire.contains("__view_image"),
            "round {round} carries the image key"
        );
        let results = tool_results(request);
        assert_eq!(results.len(), round);
        for result in results {
            assert_eq!(
                result.result.matches(NOTICE).count(),
                1,
                "{}",
                result.result
            );
            assert!(result.result.starts_with(ACTION_TEXT), "{}", result.result);
            assert_eq!(result.result.lines().count(), 2, "one notice line only");
        }
    }
}

#[tokio::test]
async fn text_only_engine_is_not_offered_screenshot_only_tools() {
    let (requests, _) = blind_run().await;
    for request in &requests {
        let names = tool_names(request);
        assert!(names.contains(&"browser_use_click"), "{names:?}");
        for hidden in SCREENSHOT_ONLY_TOOLS {
            assert!(!names.contains(&hidden), "{hidden} advertised: {names:?}");
        }
    }
}

#[tokio::test]
async fn persisted_transcript_keeps_the_image_and_never_the_notice() {
    let (_, persisted) = blind_run().await;
    assert_eq!(persisted.len(), 30);
    for result in persisted {
        assert_eq!(
            result.result, ACTION_TEXT,
            "the notice is a request-copy detail"
        );
        let payload = result.display_payload.expect("persisted display payload");
        assert_eq!(payload["__view_image"]["image_url"], IMAGE_URL);
    }
}

fn data_urls(request: &AgentInferenceRequest) -> usize {
    serde_json::to_string(&request.transcript)
        .unwrap()
        .matches("data:image")
        .count()
}

#[tokio::test]
async fn engine_that_forwards_images_keeps_the_payload_and_the_screenshot_tools() {
    let engine = ScriptedEngine::seeing("mock-vision", "browser_use_click", 30);
    let mut harness = harness(std::slice::from_ref(&engine));
    harness
        .run("mock-vision", "scripted-model", "click through the page")
        .await;

    let requests = engine.requests();
    assert_eq!(requests.len(), 31);
    for (round, request) in requests.iter().enumerate() {
        assert_eq!(data_urls(request), round, "one image per earlier result");
        for result in tool_results(request) {
            assert_eq!(result.result, ACTION_TEXT, "no notice when it is shown");
        }
        let names = tool_names(request);
        for shown in SCREENSHOT_ONLY_TOOLS {
            assert!(names.contains(&shown), "{shown} missing: {names:?}");
        }
    }
}

#[tokio::test]
async fn the_decision_follows_the_model_the_engine_is_asked_about() {
    let engine = Arc::new(ScriptedEngine {
        id: "mock-mixed",
        tool: "browser_use_click",
        forwards_images_for: Some("vision-model"),
        steps: AtomicUsize::new(1),
        requests: Mutex::new(Vec::new()),
    });
    let mut harness = harness(std::slice::from_ref(&engine));
    harness.run("mock-mixed", "text-model", "first").await;
    harness.run("mock-mixed", "vision-model", "second").await;

    let requests = engine.requests();
    // First turn on the text model: request 1 is the tool round's follow-up.
    assert_eq!(data_urls(&requests[1]), 0);
    assert!(!tool_names(&requests[1]).contains(&"chrome_screenshot"));
    // Second turn on the vision model sees the first turn's image as well as its own.
    let last = requests.last().unwrap();
    assert_eq!(data_urls(last), 2);
    assert!(tool_names(last).contains(&"chrome_screenshot"));
}

#[tokio::test]
async fn switching_engines_mid_thread_replays_the_same_history() {
    let seeing = ScriptedEngine::seeing("mock-vision", "chrome_screenshot", 3);
    let blind = ScriptedEngine::new("mock-blind", 0);
    let mut harness = harness(&[seeing.clone(), blind.clone()]);

    // Turn 1: the image-capable engine calls a screenshot-only tool three times.
    harness
        .run("mock-vision", "scripted-model", "look at the page")
        .await;
    // Turn 2: same thread, an engine that cannot receive images. It is not
    // offered the tool its history calls, and must still get that history.
    seeing.set_steps(0);
    let blind_tokens = harness
        .run("mock-blind", "scripted-model", "what next?")
        .await;
    // Turn 3: back to the image-capable engine, with the same user text length.
    let seeing_tokens = harness
        .run("mock-vision", "scripted-model", "what next?")
        .await;

    let replayed = &blind.requests()[0];
    assert!(!tool_names(replayed).contains(&"chrome_screenshot"));
    let calls = replayed
        .transcript
        .iter()
        .filter(|item| {
            matches!(item, TranscriptItem::ToolCall(call) if call.name == "chrome_screenshot")
        })
        .count();
    assert_eq!(calls, 3, "history keeps the calls it made to a hidden tool");
    let results = tool_results(replayed);
    assert_eq!(results.len(), 3);
    assert_eq!(data_urls(replayed), 0);
    for result in results {
        assert_eq!(result.name.as_deref(), Some("chrome_screenshot"));
        assert_eq!(result.result.matches(NOTICE).count(), 1);
    }

    let back = seeing.requests().pop().unwrap();
    assert_eq!(
        data_urls(&back),
        3,
        "the image-capable engine sees them again"
    );
    for result in tool_results(&back) {
        assert_eq!(result.result, ACTION_TEXT);
    }
    assert!(tool_names(&back).contains(&"chrome_screenshot"));

    // Accounting: the same three results cost 3 x 1,600 more where the images
    // are sent. The two prompts differ only by a few characters of text.
    let extra = seeing_tokens - blind_tokens;
    assert!(
        (4_800..4_900).contains(&extra),
        "{blind_tokens} vs {seeing_tokens}"
    );

    let persisted = harness.persisted_tool_results().await;
    assert_eq!(persisted.len(), 3);
    for result in persisted {
        assert_eq!(result.result, ACTION_TEXT);
        assert_eq!(
            result.display_payload.unwrap()["__view_image"]["image_url"],
            IMAGE_URL
        );
    }
}
