//! `roder thread ...` CLI: native worktree conversation forks (roadmap
//! phase 90). These are Roder conversation/workspace forks backed by local
//! Git worktrees — not GitHub repository forks.

use std::sync::Arc;

use roder_app_server::{AppServer, LocalAppClient};
use roder_protocol::{
    JsonRpcRequest, ThreadForkStatusParams, ThreadForkStatusResult, ThreadForkWorktreeParams,
    ThreadForkWorktreeResult, ThreadRemoveWorktreeForkParams, ThreadRemoveWorktreeForkResult,
};

use crate::{CliOptions, build_runtime_from_config, decode_response};

pub(crate) async fn run_thread_cli(args: &[String]) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        Some("fork-worktree") => fork_worktree(&args[1..]).await,
        Some("fork-status") => fork_status(&args[1..]).await,
        Some("remove-worktree-fork") => remove_worktree_fork(&args[1..]).await,
        _ => {
            print_thread_help();
            Ok(())
        }
    }
}

fn print_thread_help() {
    println!(
        "Usage:\n  roder thread fork-worktree <thread-id> --name <name> [--from-turn <turn-id>]\n  roder thread fork-status <thread-id>\n  roder thread remove-worktree-fork <thread-id> --confirm-path <worktree-path>\n\nForks the conversation into a child thread backed by an isolated local Git\nworktree (a Roder conversation/workspace fork, not a GitHub repository fork).\nRemoval is destructive and requires the exact worktree path as confirmation."
    );
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

async fn fork_worktree(args: &[String]) -> anyhow::Result<()> {
    let Some(thread_id) = args.first().filter(|arg| !arg.starts_with('-')) else {
        anyhow::bail!("usage: roder thread fork-worktree <thread-id> --name <name>");
    };
    let Some(name) = flag_value(args, "--name") else {
        anyhow::bail!("--name is required");
    };
    let client = client().await?;
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("thread/fork_worktree")),
            method: "thread/fork_worktree".to_string(),
            params: Some(serde_json::to_value(ThreadForkWorktreeParams {
                thread_id: thread_id.clone(),
                name,
                from_turn_id: flag_value(args, "--from-turn"),
            })?),
        })
        .await;
    let result = decode_response::<ThreadForkWorktreeResult>(res)?;
    println!("thread\t{}", result.thread.id);
    println!("worktree\t{}", result.fork.worktree_path);
    println!("branch\t{}", result.fork.branch);
    println!("commit\t{}", result.fork.source_commit);
    for warning in result.warnings {
        eprintln!("warning: {warning}");
    }
    Ok(())
}

async fn fork_status(args: &[String]) -> anyhow::Result<()> {
    let Some(thread_id) = args.first() else {
        anyhow::bail!("usage: roder thread fork-status <thread-id>");
    };
    let client = client().await?;
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("thread/fork_status")),
            method: "thread/fork_status".to_string(),
            params: Some(serde_json::to_value(ThreadForkStatusParams {
                thread_id: thread_id.clone(),
            })?),
        })
        .await;
    let result = decode_response::<ThreadForkStatusResult>(res)?;
    match result.fork {
        Some(fork) => {
            println!("fork\t{}", fork.fork_id);
            println!("status\t{:?}", fork.status);
            println!("worktree\t{}", fork.worktree_path);
            println!("branch\t{}", fork.branch);
            if let Some(parent) = result.parent_thread_id {
                println!("parent\t{parent}");
            }
            if result.worktree_missing {
                eprintln!(
                    "warning: the worktree directory is missing; restore it or run \
                     `roder thread remove-worktree-fork {thread_id} --confirm-path {}`",
                    fork.worktree_path
                );
            }
        }
        None => println!("thread {thread_id} is not a worktree fork"),
    }
    Ok(())
}

async fn remove_worktree_fork(args: &[String]) -> anyhow::Result<()> {
    let Some(thread_id) = args.first().filter(|arg| !arg.starts_with('-')) else {
        anyhow::bail!(
            "usage: roder thread remove-worktree-fork <thread-id> --confirm-path <worktree-path>"
        );
    };
    let Some(confirm_path) = flag_value(args, "--confirm-path") else {
        anyhow::bail!(
            "--confirm-path is required: removal is destructive and must name the exact \
             worktree path (see `roder thread fork-status`)"
        );
    };
    let client = client().await?;
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("thread/remove_worktree_fork")),
            method: "thread/remove_worktree_fork".to_string(),
            params: Some(serde_json::to_value(ThreadRemoveWorktreeForkParams {
                thread_id: thread_id.clone(),
                confirm_path,
            })?),
        })
        .await;
    let result = decode_response::<ThreadRemoveWorktreeForkResult>(res)?;
    println!(
        "removed\t{}\nbranch kept\t{}",
        result.fork.worktree_path, result.fork.branch
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_value_supports_separate_and_equals_forms() {
        let args = vec![
            "thread-1".to_string(),
            "--name".to_string(),
            "experiment".to_string(),
            "--from-turn=turn-9".to_string(),
        ];
        assert_eq!(flag_value(&args, "--name").as_deref(), Some("experiment"));
        assert_eq!(flag_value(&args, "--from-turn").as_deref(), Some("turn-9"));
        assert_eq!(flag_value(&args, "--confirm-path"), None);
    }
}
