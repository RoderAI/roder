//! `roder thread fork*` and `roder forks ...` CLI (roadmap phases 90 + 81).
//! These are Roder conversation/workspace forks backed by local fork
//! providers (`git-worktree`, `rift`) — not GitHub repository forks.
//! Removal is destructive and requires the exact workspace path.

use std::sync::Arc;

use roder_api::lifecycle::TurnLifecycleSnapshot;
use roder_app_server::{AppServer, LocalAppClient};
use roder_protocol::{
    ForksCreateParams, ForksCreateResult, ForksListParams, ForksListResult,
    ForksProvidersListResult, ForksRemoveParams, ForksRemoveResult, JsonRpcRequest,
    ThreadForkParams, ThreadForkResult, ThreadForkStatusParams, ThreadForkStatusResult,
    ThreadReadParams, ThreadReadResult, ThreadRemoveForkParams, ThreadRemoveForkResult,
};

use crate::{CliOptions, build_runtime_from_config, decode_response};

pub(crate) async fn run_thread_cli(args: &[String]) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        Some("fork") => thread_fork(&args[1..]).await,
        Some("fork-status") => thread_fork_status(&args[1..]).await,
        Some("lifecycle") => thread_lifecycle(&args[1..]).await,
        Some("remove-fork") => thread_remove_fork(&args[1..]).await,
        _ => {
            println!(
                "Usage:\n  roder thread fork <thread-id> --name <name> [--from-turn <turn-id>] [--provider <id>]\n  roder thread fork-status <thread-id>\n  roder thread lifecycle <thread-id> [--json]\n  roder thread remove-fork <thread-id> --confirm-path <workspace-path>\n\nForks the conversation into a child thread backed by an isolated workspace\nfork (a Roder conversation/workspace fork, not a GitHub repository fork).\nUse `thread lifecycle` to inspect durable turn recovery records. Providers: see\n`roder forks providers`. Removal is destructive and requires the exact fork\nworkspace path as confirmation."
            );
            Ok(())
        }
    }
}

pub(crate) async fn run_forks_cli(args: &[String]) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        Some("providers") => forks_providers().await,
        Some("list") => forks_list(&args[1..]).await,
        Some("create") => forks_create(&args[1..]).await,
        Some("remove") => forks_remove(&args[1..]).await,
        _ => {
            println!(
                "Usage:\n  roder forks providers\n  roder forks list [--source <path>] [--provider <id>]\n  roder forks create --name <name> [--source <path>] [--provider <id>]\n  roder forks remove <fork-id> --confirm-workspace <path> [--provider <id>]\n\nWorkspace forks are isolated writable copies of a source workspace\n(git-worktree by default; rift for copy-on-write snapshots)."
            );
            Ok(())
        }
    }
}

async fn client() -> anyhow::Result<LocalAppClient> {
    let (runtime, _) = build_runtime_from_config(CliOptions::default()).await?;
    Ok(LocalAppClient::new(Arc::new(AppServer::new(runtime))))
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == flag {
            return iter.next().cloned();
        }
        if let Some(value) = arg.strip_prefix(&format!("{flag}=")) {
            return Some(value.to_string());
        }
    }
    None
}

fn source_or_cwd(args: &[String]) -> anyhow::Result<String> {
    match flag_value(args, "--source") {
        Some(source) => Ok(source),
        None => Ok(std::env::current_dir()?.display().to_string()),
    }
}

async fn call<T: serde::de::DeserializeOwned>(
    client: &LocalAppClient,
    method: &str,
    params: serde_json::Value,
) -> anyhow::Result<T> {
    let response = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(method)),
            method: method.to_string(),
            params: Some(params),
        })
        .await;
    decode_response(response)
}

async fn thread_fork(args: &[String]) -> anyhow::Result<()> {
    let Some(thread_id) = args.first().filter(|arg| !arg.starts_with('-')) else {
        anyhow::bail!("usage: roder thread fork <thread-id> --name <name>");
    };
    let Some(name) = flag_value(args, "--name") else {
        anyhow::bail!("--name is required");
    };
    let client = client().await?;
    let result: ThreadForkResult = call(
        &client,
        "thread/fork",
        serde_json::to_value(ThreadForkParams {
            thread_id: thread_id.clone(),
            name,
            from_turn_id: flag_value(args, "--from-turn"),
            provider: flag_value(args, "--provider"),
            provider_config: None,
        })?,
    )
    .await?;
    println!("thread\t{}", result.thread.id);
    println!("provider\t{}", result.fork.provider_id);
    println!("workspace\t{}", result.fork.workspace.display());
    if let Some(branch) = &result.fork.provenance.branch {
        println!("branch\t{branch}");
    }
    for warning in result.warnings {
        eprintln!("warning: {warning}");
    }
    Ok(())
}

async fn thread_fork_status(args: &[String]) -> anyhow::Result<()> {
    let Some(thread_id) = args.first() else {
        anyhow::bail!("usage: roder thread fork-status <thread-id>");
    };
    let client = client().await?;
    let result: ThreadForkStatusResult = call(
        &client,
        "thread/fork_status",
        serde_json::to_value(ThreadForkStatusParams {
            thread_id: thread_id.clone(),
        })?,
    )
    .await?;
    match result.fork {
        Some(fork) => {
            println!("fork\t{}", fork.id);
            println!("provider\t{}", fork.provider_id);
            println!("status\t{:?}", fork.status);
            println!("workspace\t{}", fork.workspace.display());
            if let Some(parent) = result.parent_thread_id {
                println!("parent\t{parent}");
            }
            if result.workspace_missing {
                eprintln!(
                    "warning: the fork workspace is missing; restore it or run \
                     `roder thread remove-fork {thread_id} --confirm-path {}`",
                    fork.workspace.display()
                );
            }
        }
        None => println!("thread {thread_id} is not a workspace fork"),
    }
    Ok(())
}

async fn thread_lifecycle(args: &[String]) -> anyhow::Result<()> {
    let Some(thread_id) = args.first().filter(|arg| !arg.starts_with('-')) else {
        anyhow::bail!("usage: roder thread lifecycle <thread-id> [--json]");
    };
    let json = args.iter().any(|arg| arg == "--json");
    let client = client().await?;
    let result: ThreadReadResult = call(
        &client,
        "thread/read",
        serde_json::to_value(ThreadReadParams {
            thread_id: thread_id.clone(),
            include_turns: false,
        })?,
    )
    .await?;

    if result.thread.is_none() {
        anyhow::bail!("thread {thread_id:?} was not found");
    }

    if json {
        println!(
            "{}",
            format_thread_lifecycle_json(thread_id, &result.lifecycle)?
        );
        return Ok(());
    }

    print!("{}", format_thread_lifecycle_text(&result.lifecycle));
    Ok(())
}

fn format_thread_lifecycle_json(
    thread_id: &str,
    lifecycle: &TurnLifecycleSnapshot,
) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "threadId": thread_id,
        "lifecycle": lifecycle,
    }))?)
}

fn format_thread_lifecycle_text(lifecycle: &TurnLifecycleSnapshot) -> String {
    let mut output = format!(
        "corrupt_lifecycle_records\t{}\n",
        lifecycle.corrupt_record_count
    );
    if lifecycle.records.is_empty() {
        output.push_str("no lifecycle records\n");
        return output;
    }

    output.push_str("turn\tstate\tcleanup\townership\treason\ttimestamp\n");
    for record in &lifecycle.records {
        let reason = record
            .reason
            .map(|reason| format!("{reason:?}"))
            .unwrap_or_else(|| "-".to_string());
        output.push_str(&format!(
            "{}\t{:?}\t{:?}\t{:?}\t{}\t{}\n",
            record.turn_id,
            record.state,
            record.cleanup,
            record.ownership,
            reason,
            record.timestamp
        ));
    }
    output
}

async fn thread_remove_fork(args: &[String]) -> anyhow::Result<()> {
    let Some(thread_id) = args.first().filter(|arg| !arg.starts_with('-')) else {
        anyhow::bail!("usage: roder thread remove-fork <thread-id> --confirm-path <path>");
    };
    let Some(confirm_path) = flag_value(args, "--confirm-path") else {
        anyhow::bail!(
            "--confirm-path is required: removal is destructive and must name the exact \
             fork workspace path (see `roder thread fork-status`)"
        );
    };
    let client = client().await?;
    let result: ThreadRemoveForkResult = call(
        &client,
        "thread/remove_fork",
        serde_json::to_value(ThreadRemoveForkParams {
            thread_id: thread_id.clone(),
            confirm_path,
        })?,
    )
    .await?;
    println!("removed\t{}", result.fork.workspace.display());
    if let Some(branch) = &result.fork.provenance.branch {
        println!("branch kept\t{branch}");
    }
    Ok(())
}

async fn forks_providers() -> anyhow::Result<()> {
    let client = client().await?;
    let result: ForksProvidersListResult =
        call(&client, "forks/providers/list", serde_json::json!({})).await?;
    println!("{:<14} {:<18} capabilities", "id", "name");
    for provider in result.providers {
        let mut labels = Vec::new();
        let caps = &provider.capabilities;
        if caps.create {
            labels.push("create");
        }
        if caps.copy_on_write {
            labels.push("cow");
        }
        if caps.remote_compute {
            labels.push("remote");
        }
        println!(
            "{:<14} {:<18} {}",
            provider.id,
            provider.display_name,
            labels.join(",")
        );
    }
    Ok(())
}

async fn forks_list(args: &[String]) -> anyhow::Result<()> {
    let client = client().await?;
    let result: ForksListResult = call(
        &client,
        "forks/list",
        serde_json::to_value(ForksListParams {
            source_workspace: source_or_cwd(args)?,
            provider: flag_value(args, "--provider"),
        })?,
    )
    .await?;
    if result.forks.is_empty() {
        println!("no forks");
        return Ok(());
    }
    for fork in result.forks {
        println!(
            "{:?}\t{}\t{}",
            fork.status,
            fork.provider_id,
            fork.workspace.display()
        );
    }
    Ok(())
}

async fn forks_create(args: &[String]) -> anyhow::Result<()> {
    let client = client().await?;
    let result: ForksCreateResult = call(
        &client,
        "forks/create",
        serde_json::to_value(ForksCreateParams {
            source_workspace: source_or_cwd(args)?,
            name: flag_value(args, "--name"),
            provider: flag_value(args, "--provider"),
            provider_config: None,
        })?,
    )
    .await?;
    println!("fork\t{}", result.fork.id);
    println!("workspace\t{}", result.fork.workspace.display());
    Ok(())
}

async fn forks_remove(args: &[String]) -> anyhow::Result<()> {
    let Some(fork_id) = args.first().filter(|arg| !arg.starts_with('-')) else {
        anyhow::bail!("usage: roder forks remove <fork-id> --confirm-workspace <path>");
    };
    let Some(confirm_workspace) = flag_value(args, "--confirm-workspace") else {
        anyhow::bail!(
            "--confirm-workspace is required: removal is destructive and must name the exact \
             fork workspace path"
        );
    };
    let client = client().await?;
    let result: ForksRemoveResult = call(
        &client,
        "forks/remove",
        serde_json::to_value(ForksRemoveParams {
            fork_id: fork_id.clone(),
            provider: flag_value(args, "--provider"),
            confirm_workspace,
        })?,
    )
    .await?;
    println!("removed\t{}", result.removed);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::lifecycle::{
        TurnCleanupOwnership, TurnCleanupState, TurnLifecycleReason, TurnLifecycleRecord,
        TurnLifecycleState,
    };

    #[test]
    fn forks_flag_parsing_supports_separate_and_equals_forms() {
        let args = vec![
            "thread-1".to_string(),
            "--name".to_string(),
            "experiment".to_string(),
            "--provider=rift".to_string(),
        ];
        assert_eq!(flag_value(&args, "--name").as_deref(), Some("experiment"));
        assert_eq!(flag_value(&args, "--provider").as_deref(), Some("rift"));
        assert_eq!(flag_value(&args, "--confirm-path"), None);
    }

    #[test]
    fn lifecycle_inspection_formats_machine_readable_and_text_output() {
        let lifecycle = TurnLifecycleSnapshot {
            records: vec![TurnLifecycleRecord {
                thread_id: "thread-lifecycle".to_string(),
                turn_id: "turn-lifecycle".to_string(),
                state: TurnLifecycleState::RecoveryNeeded,
                cleanup: TurnCleanupState::TimedOut,
                reason: Some(TurnLifecycleReason::RuntimeRestart),
                ownership: TurnCleanupOwnership::RuntimeTaskOnly,
                timestamp: time::OffsetDateTime::UNIX_EPOCH,
            }],
            corrupt_record_count: 2,
        };

        let json = format_thread_lifecycle_json("thread-lifecycle", &lifecycle).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&json).unwrap()["threadId"],
            "thread-lifecycle"
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&json).unwrap()["lifecycle"]["corruptRecordCount"],
            2
        );

        let text = format_thread_lifecycle_text(&lifecycle);
        assert!(text.contains("corrupt_lifecycle_records\t2"));
        assert!(text.contains("turn\tstate\tcleanup\townership\treason\ttimestamp"));
        assert!(text.contains("turn-lifecycle\tRecoveryNeeded\tTimedOut"));
        assert!(text.contains("RuntimeRestart"));
    }
}
