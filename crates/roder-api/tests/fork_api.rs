//! Fork API contract tests (roadmap phase 81, Task 1): serde shapes,
//! registry registration/lookup, and duplicate-provider validation.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use roder_api::extension::{ExtensionRegistryBuilder, ProvidedService};
use roder_api::forks::*;
use time::OffsetDateTime;

struct FakeForkProvider {
    id: &'static str,
}

#[async_trait::async_trait]
impl ForkProvider for FakeForkProvider {
    fn descriptor(&self) -> ForkProviderDescriptor {
        ForkProviderDescriptor {
            id: self.id.to_string(),
            display_name: "Fake".to_string(),
            capabilities: ForkCapabilities {
                create: true,
                list: true,
                remove: true,
                resume: true,
                ..ForkCapabilities::default()
            },
        }
    }

    async fn create_fork(&self, request: ForkRequest) -> anyhow::Result<WorkspaceFork> {
        Ok(WorkspaceFork {
            id: "fork-1".to_string(),
            provider_id: self.id.to_string(),
            source_workspace: request.source_workspace.clone(),
            workspace: request.source_workspace.join("fork-1"),
            status: ForkStatus::Active,
            provenance: ForkProvenance::at(OffsetDateTime::UNIX_EPOCH),
            cleanup: ForkCleanupPolicy::Explicit,
            metadata: serde_json::json!({}),
        })
    }

    async fn list_forks(&self, _source: &Path) -> anyhow::Result<Vec<WorkspaceFork>> {
        Ok(Vec::new())
    }

    async fn resume_fork(&self, id: &ForkId) -> anyhow::Result<WorkspaceFork> {
        anyhow::bail!("unknown fork {id}")
    }

    async fn remove_fork(
        &self,
        id: &ForkId,
        policy: RemoveForkPolicy,
    ) -> anyhow::Result<RemoveForkResult> {
        Ok(RemoveForkResult {
            id: id.clone(),
            removed: true,
            workspace: policy.confirm_workspace,
        })
    }
}

#[test]
fn fork_types_round_trip_with_camel_case_wire_names() {
    let fork = WorkspaceFork {
        id: "/repo/.roder/worktrees/x".to_string(),
        provider_id: "git-worktree".to_string(),
        source_workspace: PathBuf::from("/repo"),
        workspace: PathBuf::from("/repo/.roder/worktrees/x"),
        status: ForkStatus::Active,
        provenance: ForkProvenance {
            branch: Some("roder/fork/x".to_string()),
            source_branch: Some("main".to_string()),
            source_commit: Some("abc".to_string()),
            snapshot_id: None,
            session_id: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
        },
        cleanup: ForkCleanupPolicy::Explicit,
        metadata: serde_json::json!({ "note": "isolated" }),
    };
    let json = serde_json::to_value(&fork).unwrap();
    assert_eq!(json["providerId"], "git-worktree");
    assert_eq!(json["sourceWorkspace"], "/repo");
    assert_eq!(json["status"], "active");
    assert_eq!(json["cleanup"], "explicit");
    assert_eq!(json["provenance"]["sourceBranch"], "main");
    let round_trip: WorkspaceFork = serde_json::from_value(json).unwrap();
    assert_eq!(round_trip, fork);

    // Requests default policy fails closed on dirty sources.
    let request: ForkRequest = serde_json::from_value(serde_json::json!({
        "sourceWorkspace": "/repo",
        "reason": "experiment",
        "providerConfig": {}
    }))
    .unwrap();
    assert!(!request.policy.allow_dirty_source);
    assert_eq!(request.reason, ForkReason::Experiment);
}

#[tokio::test]
async fn registry_registers_and_resolves_fork_providers() {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.manifest(roder_api::extension::ExtensionManifest {
        id: "ext-fake-fork".to_string(),
        name: "Fake Fork".to_string(),
        version: semver::Version::new(0, 1, 0),
        api_version: "0.1.0".to_string(),
        description: None,
        provides: vec![ProvidedService::ForkProvider("fake-fork".to_string())],
        required_capabilities: Vec::new(),
    });
    builder.fork_provider(Arc::new(FakeForkProvider { id: "fake-fork" }));
    let registry = builder.build().unwrap();

    assert!(
        registry
            .provided_services()
            .contains(&ProvidedService::ForkProvider("fake-fork".to_string()))
    );
    let provider = registry.fork_provider("fake-fork").expect("lookup by id");
    assert!(provider.descriptor().capabilities.create);
    assert!(registry.fork_provider("missing").is_none());

    let fork = provider
        .create_fork(ForkRequest {
            source_workspace: PathBuf::from("/src"),
            name: Some("x".to_string()),
            reason: ForkReason::SubagentLane,
            policy: ForkPolicy::default(),
            provider_config: serde_json::json!({}),
        })
        .await
        .unwrap();
    assert_eq!(fork.status, ForkStatus::Active);
}

#[test]
fn duplicate_fork_provider_ids_fail_registry_validation() {
    let manifest = |suffix: &str| roder_api::extension::ExtensionManifest {
        id: format!("ext-{suffix}"),
        name: format!("Ext {suffix}"),
        version: semver::Version::new(0, 1, 0),
        api_version: "0.1.0".to_string(),
        description: None,
        provides: vec![ProvidedService::ForkProvider("dup-fork".to_string())],
        required_capabilities: Vec::new(),
    };
    let mut builder = ExtensionRegistryBuilder::new();
    builder.manifest(manifest("a"));
    builder.manifest(manifest("b"));
    builder.fork_provider(Arc::new(FakeForkProvider { id: "dup-fork" }));
    let error = match builder.build() {
        Ok(_) => panic!("duplicate fork provider ids must fail validation"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("ForkProvider(dup-fork)"), "{error}");
}
