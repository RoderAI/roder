//! Tests for [`super`]: the review runner is exercised end to end against a
//! capture engine that records what the reviewer was told and answers with a
//! canned structured review.

use std::sync::Mutex as StdMutex;

use futures::stream;
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, InferenceCapabilities, InferenceEngine,
    InferenceEventStream, InferenceProviderContext, InferenceTurnContext, MessageDelta,
};
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;

use crate::review::test_vcs::StubVcs;
use crate::runtime::RuntimeConfig;

use super::*;

const WORKSPACE: &str = "/tmp/roder-review-test";

const CANNED_REVIEW: &str = r#"{
  "findings": [
    {
      "title": "[P1] Off-by-one in the loop bound",
      "body": "The loop overruns the slice.",
      "confidenceScore": 0.8,
      "priority": "p1",
      "codeLocation": {
        "absoluteFilePath": "/tmp/roder-review-test/src/lib.rs",
        "lineRange": { "start": 10, "end": 12 }
      }
    }
  ],
  "overallCorrectness": "patch is incorrect",
  "overallExplanation": "One blocking bug.",
  "overallConfidenceScore": 0.7
}"#;

/// Records every inference request and answers with a canned review, so the
/// test can assert both what the reviewer was told and what came back.
struct ReviewCaptureEngine {
    requests: Arc<StdMutex<Vec<AgentInferenceRequest>>>,
}

#[async_trait::async_trait]
impl InferenceEngine for ReviewCaptureEngine {
    fn id(&self) -> String {
        roder_api::catalog::PROVIDER_MOCK.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::coding_agent_default()
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<roder_api::inference::ModelDescriptor>> {
        Ok(roder_api::catalog::models_for_provider(
            roder_api::catalog::PROVIDER_MOCK,
            true,
        ))
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        self.requests.lock().unwrap().push(request);
        Ok(Box::pin(stream::iter(vec![
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: CANNED_REVIEW.to_string(),
                phase: None,
            })),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("stop".to_string()),
                provider_response_id: None,
            })),
        ])))
    }
}

struct Harness {
    runtime: Arc<Runtime>,
    requests: Arc<StdMutex<Vec<AgentInferenceRequest>>>,
    thread_root: PathBuf,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.thread_root);
    }
}

fn harness(vcs: Option<StubVcs>) -> Harness {
    let requests = Arc::new(StdMutex::new(Vec::new()));
    let thread_root = std::env::temp_dir().join(format!("roder-review-{}", uuid::Uuid::new_v4()));
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(ReviewCaptureEngine {
        requests: requests.clone(),
    }));
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: thread_root.clone(),
    }));
    if let Some(vcs) = vcs {
        builder.version_control_provider(Arc::new(vcs));
    }
    let runtime = Arc::new(
        Runtime::new(
            builder.build().expect("build registry"),
            RuntimeConfig {
                workspace: Some(WORKSPACE.to_string()),
                ..RuntimeConfig::default()
            },
        )
        .expect("build runtime"),
    );
    Harness {
        runtime,
        requests,
        thread_root,
    }
}

async fn parent_thread(runtime: &Arc<Runtime>) -> ThreadId {
    runtime
        .create_thread_with(CreateThreadRequest {
            title: None,
            workspace: WORKSPACE.to_string(),
            workspace_id: None,
            root_id: None,
            provider: None,
            model: None,
            selection_mode: None,
            tool_allowlist: Vec::new(),
            developer_instructions: None,
            external_tools: Vec::new(),
            runner: None,
        })
        .await
        .expect("create parent thread")
        .thread_id
}

fn start_request(thread_id: &ThreadId) -> StartReviewRequest {
    StartReviewRequest {
        thread_id: thread_id.clone(),
        target: ReviewTarget::UncommittedChanges,
        workspace: WORKSPACE.to_string(),
        provider_override: None,
        model_override: None,
    }
}

#[tokio::test]
async fn review_runs_on_a_detached_read_only_child_thread() {
    let harness = harness(Some(StubVcs::new(Some("origin/main"), Some("basesha"))));
    let parent = parent_thread(&harness.runtime).await;

    let completion = harness
        .runtime
        .run_review(start_request(&parent))
        .await
        .expect("run review");

    assert_ne!(completion.handle.review_thread_id, parent);
    let metadata = harness
        .runtime
        .load_thread_metadata(&completion.handle.review_thread_id)
        .await
        .expect("load review thread")
        .expect("review thread exists");
    assert_eq!(metadata.root_id.as_deref(), Some(parent.as_str()));
    assert_eq!(
        metadata.tool_allowlist,
        REVIEW_TOOL_ALLOWLIST
            .iter()
            .map(|name| (*name).to_string())
            .collect::<Vec<_>>()
    );

    let requests = harness.requests.lock().unwrap();
    let system = requests
        .first()
        .expect("review turn issued an inference request")
        .instructions
        .system
        .as_deref()
        .expect("review turn replaces the system prompt");
    assert!(system.contains("Review guidelines"));
    assert!(system.contains("\"overallCorrectness\""));
}

#[tokio::test]
async fn review_parses_findings_emits_events_and_echoes_into_the_parent() {
    let harness = harness(Some(StubVcs::new(Some("origin/main"), Some("basesha"))));
    let parent = parent_thread(&harness.runtime).await;
    let mut events = harness.runtime.subscribe_events();

    let completion = harness
        .runtime
        .run_review(start_request(&parent))
        .await
        .expect("run review");

    assert_eq!(completion.output.findings.len(), 1);
    assert_eq!(
        completion.output.findings[0].priority,
        roder_api::review::ReviewPriority::P1
    );

    let mut started = None;
    let mut completed = None;
    let mut echoed_user_action = false;
    let mut echoed_summary = false;
    while let Ok(envelope) = events.try_recv() {
        let parent_thread = envelope.thread_id.as_deref() == Some(parent.as_str());
        match envelope.event {
            RoderEvent::ReviewStarted(event) => started = Some(event),
            RoderEvent::ReviewCompleted(event) => completed = Some(event),
            RoderEvent::TranscriptItemAppended(event) if parent_thread => match &event.item {
                Some(TranscriptItem::UserMessage(message))
                    if message.text.contains("<action>review</action>") =>
                {
                    echoed_user_action = true;
                }
                Some(TranscriptItem::AssistantMessage(message))
                    if message.text.contains("- [P1] Off-by-one") =>
                {
                    echoed_summary = true;
                }
                _ => {}
            },
            _ => {}
        }
    }

    let started = started.expect("review started event");
    assert_eq!(started.thread_id, parent);
    assert_eq!(started.review_id, completion.handle.review_id);
    assert_eq!(started.label, "current changes");

    let completed = completed.expect("review completed event");
    assert_eq!(completed.thread_id, parent);
    assert_eq!(
        completed.review_thread_id,
        completion.handle.review_thread_id
    );
    assert_eq!(completed.output.findings.len(), 1);
    assert!(echoed_user_action, "parent thread must record the review");
    assert!(echoed_summary, "parent thread must record the findings");

    let record = harness
        .runtime
        .review_record(&completion.handle.review_id)
        .await
        .expect("review record");
    assert_eq!(record.parent_thread_id, parent);
    assert_eq!(record.request.base_sha.as_deref(), Some("basesha"));
    assert_eq!(record.workspace_root, PathBuf::from(WORKSPACE));
    assert_eq!(record.output.findings.len(), 1);
    assert_eq!(
        harness
            .runtime
            .latest_review_record()
            .await
            .expect("latest record")
            .review_id,
        completion.handle.review_id
    );
}

#[tokio::test]
async fn review_requires_a_version_control_provider() {
    let harness = harness(None);
    let parent = parent_thread(&harness.runtime).await;

    let error = harness
        .runtime
        .run_review(start_request(&parent))
        .await
        .expect_err("review without a provider must fail");
    assert!(
        error.to_string().contains("no version control provider"),
        "unexpected error: {error}"
    );
}
