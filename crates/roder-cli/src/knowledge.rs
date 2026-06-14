//! `roder knowledge` CLI: manage the project knowledge base through the
//! app-server `knowledge/*` methods (roadmap phase 93).

use std::sync::Arc;

use roder_api::knowledge::{KnowledgeKind, KnowledgeLinkType, KnowledgeStatus};
use roder_api::memory::MemoryScope;
use roder_app_server::{AppServer, LocalAppClient};
use roder_protocol::{
    JsonRpcRequest, KnowledgeDeleteParams, KnowledgeDeleteResult, KnowledgeLinkSetParams,
    KnowledgeListParams, KnowledgeListResult, KnowledgeReadParams, KnowledgeReadResult,
    KnowledgeRevisionsParams, KnowledgeRevisionsResult, KnowledgeSaveParams, KnowledgeSaveResult,
    KnowledgeSearchParams, KnowledgeSearchResults, KnowledgeUpdateParams,
};

use crate::{CliOptions, build_runtime_from_config, decode_response};

const USAGE: &str = "usage: roder knowledge <list|read|search|save|update|delete|link|revisions>\n\
  roder knowledge list [--scope project|global|project:<id>] [--kind KIND] [--tag TAG] [--status STATUS]\n\
  roder knowledge read ID [--revision N]\n\
  roder knowledge search TEXT [--scope ...] [--kind KIND] [--include-global]\n\
  roder knowledge save --kind KIND --title TITLE [BODY] [--tag TAG]... [--scope ...] (body from arg or stdin)\n\
  roder knowledge update ID [--title TITLE] [--status STATUS] [--tag TAG]... [BODY]\n\
  roder knowledge delete ID\n\
  roder knowledge link FROM TO --type relates_to|supersedes|derived_from|contradicts|duplicates [--remove]\n\
  roder knowledge revisions ID";

pub(crate) async fn run_knowledge_cli(args: &[String]) -> anyhow::Result<()> {
    let (runtime, _) = build_runtime_from_config(CliOptions::default()).await?;
    let client = LocalAppClient::new(Arc::new(AppServer::new(runtime)));
    match args.first().map(String::as_str) {
        Some("list") => list(&client, args).await,
        Some("read") => read(&client, args).await,
        Some("search") => search(&client, args).await,
        Some("save") => save(&client, args).await,
        Some("update") => update(&client, args).await,
        Some("delete") => delete(&client, args).await,
        Some("link") => link(&client, args).await,
        Some("revisions") => revisions(&client, args).await,
        _ => anyhow::bail!("{USAGE}"),
    }
}

async fn list(client: &LocalAppClient, args: &[String]) -> anyhow::Result<()> {
    let result: KnowledgeListResult = call(
        client,
        "knowledge/list",
        KnowledgeListParams {
            scope: Some(scope_arg(args)),
            kind: flag_value(args, "--kind").map(|kind| KnowledgeKind::parse(&kind)),
            tag: flag_value(args, "--tag"),
            status: flag_value(args, "--status")
                .map(|status| parse_status(&status))
                .transpose()?,
            include_archived: has_flag(args, "--include-archived"),
            limit: Some(100),
        },
    )
    .await?;
    for doc in result.documents {
        println!(
            "{}\t{}\t{}\t[{}] rev {}\t{}",
            doc.id,
            doc.kind,
            doc.status.as_str(),
            doc.scope.stable_id(),
            doc.revision,
            doc.title
        );
    }
    Ok(())
}

async fn read(client: &LocalAppClient, args: &[String]) -> anyhow::Result<()> {
    let Some(doc_id) = positional(args, 1) else {
        anyhow::bail!("usage: roder knowledge read ID [--revision N]");
    };
    let result: KnowledgeReadResult = call(
        client,
        "knowledge/read",
        KnowledgeReadParams {
            doc_id,
            revision: flag_value(args, "--revision")
                .map(|value| value.parse::<u32>())
                .transpose()?,
        },
    )
    .await?;
    match result.document {
        Some(doc) => {
            println!(
                "# {} ({}, {}, rev {}, {})",
                doc.title,
                doc.kind,
                doc.status.as_str(),
                doc.revision,
                doc.scope.stable_id()
            );
            if !doc.tags.is_empty() {
                println!("tags: {}", doc.tags.join(", "));
            }
            for link in &doc.links {
                println!("link: {} {}", link.link_type.as_str(), link.to);
            }
            println!("\n{}", doc.body);
        }
        None => println!("not found"),
    }
    Ok(())
}

async fn search(client: &LocalAppClient, args: &[String]) -> anyhow::Result<()> {
    let Some(text) = positional(args, 1) else {
        anyhow::bail!("usage: roder knowledge search TEXT [--scope ...] [--include-global]");
    };
    let result: KnowledgeSearchResults = call(
        client,
        "knowledge/search",
        KnowledgeSearchParams {
            scope: Some(scope_arg(args)),
            text,
            kind: flag_value(args, "--kind").map(|kind| KnowledgeKind::parse(&kind)),
            limit: Some(10),
            include_global: has_flag(args, "--include-global"),
        },
    )
    .await?;
    for matched in result.results {
        println!(
            "{:.3}\t{}\t{}\t{}\n\t{}",
            matched.score,
            matched.document.id,
            matched.document.kind,
            matched.document.title,
            matched.snippet
        );
    }
    Ok(())
}

async fn save(client: &LocalAppClient, args: &[String]) -> anyhow::Result<()> {
    let Some(kind) = flag_value(args, "--kind") else {
        anyhow::bail!("usage: roder knowledge save --kind KIND --title TITLE [BODY]");
    };
    let Some(title) = flag_value(args, "--title") else {
        anyhow::bail!("usage: roder knowledge save --kind KIND --title TITLE [BODY]");
    };
    let body = body_arg(args, 1)?;
    let result: KnowledgeSaveResult = call(
        client,
        "knowledge/save",
        KnowledgeSaveParams {
            scope: scope_arg(args),
            kind: KnowledgeKind::parse(&kind),
            title,
            tags: flag_values(args, "--tag"),
            body,
        },
    )
    .await?;
    println!("{}", result.document.id);
    Ok(())
}

async fn update(client: &LocalAppClient, args: &[String]) -> anyhow::Result<()> {
    let Some(doc_id) = positional(args, 1) else {
        anyhow::bail!("usage: roder knowledge update ID [--title TITLE] [--status STATUS] [BODY]");
    };
    let body = body_arg(args, 2).ok();
    let tags = flag_values(args, "--tag");
    let result: KnowledgeSaveResult = call(
        client,
        "knowledge/update",
        KnowledgeUpdateParams {
            doc_id,
            title: flag_value(args, "--title"),
            body,
            status: flag_value(args, "--status")
                .map(|status| parse_status(&status))
                .transpose()?,
            tags: if tags.is_empty() { None } else { Some(tags) },
        },
    )
    .await?;
    println!("{}\trev {}", result.document.id, result.document.revision);
    Ok(())
}

async fn delete(client: &LocalAppClient, args: &[String]) -> anyhow::Result<()> {
    let Some(doc_id) = positional(args, 1) else {
        anyhow::bail!("usage: roder knowledge delete ID");
    };
    let result: KnowledgeDeleteResult =
        call(client, "knowledge/delete", KnowledgeDeleteParams { doc_id }).await?;
    println!("archived: {}", result.archived);
    Ok(())
}

async fn link(client: &LocalAppClient, args: &[String]) -> anyhow::Result<()> {
    let (Some(from), Some(to), Some(link_type)) = (
        positional(args, 1),
        positional(args, 2),
        flag_value(args, "--type"),
    ) else {
        anyhow::bail!("usage: roder knowledge link FROM TO --type TYPE [--remove]");
    };
    let result: KnowledgeSaveResult = call(
        client,
        "knowledge/links/set",
        KnowledgeLinkSetParams {
            from,
            to,
            link_type: parse_link_type(&link_type)?,
            remove: has_flag(args, "--remove"),
        },
    )
    .await?;
    for link in result.document.links {
        println!("link: {} {}", link.link_type.as_str(), link.to);
    }
    Ok(())
}

async fn revisions(client: &LocalAppClient, args: &[String]) -> anyhow::Result<()> {
    let Some(doc_id) = positional(args, 1) else {
        anyhow::bail!("usage: roder knowledge revisions ID");
    };
    let result: KnowledgeRevisionsResult = call(
        client,
        "knowledge/revisions/list",
        KnowledgeRevisionsParams { doc_id },
    )
    .await?;
    for info in result.revisions {
        println!(
            "rev {}\t{}\t{}",
            info.revision, info.created_at, info.content_hash
        );
    }
    Ok(())
}

async fn call<P: serde::Serialize, T: serde::de::DeserializeOwned>(
    client: &LocalAppClient,
    method: &str,
    params: P,
) -> anyhow::Result<T> {
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(method)),
            method: method.to_string(),
            params: Some(serde_json::to_value(params)?),
        })
        .await;
    decode_response::<T>(res)
}

/// First non-flag argument at logical position `index` (subcommand = 0),
/// skipping `--flag value` pairs.
fn positional(args: &[String], index: usize) -> Option<String> {
    let mut position = 0;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg.starts_with("--") {
            if !matches!(
                arg.as_str(),
                "--include-global" | "--include-archived" | "--remove"
            ) {
                iter.next();
            }
            continue;
        }
        if position == index {
            return Some(arg.clone());
        }
        position += 1;
    }
    None
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|idx| args.get(idx + 1))
        .cloned()
}

fn flag_values(args: &[String], flag: &str) -> Vec<String> {
    let mut values = Vec::new();
    for (idx, arg) in args.iter().enumerate() {
        if arg == flag
            && let Some(value) = args.get(idx + 1)
        {
            values.push(value.clone());
        }
    }
    values
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| arg == flag)
}

/// Body from the positional argument at `index`, falling back to stdin when
/// absent and stdin is piped.
fn body_arg(args: &[String], index: usize) -> anyhow::Result<String> {
    if let Some(body) = positional(args, index) {
        return Ok(body);
    }
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        anyhow::bail!("missing body: pass it as an argument or pipe it on stdin");
    }
    let mut body = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut body)?;
    Ok(body.trim_end().to_string())
}

fn scope_arg(args: &[String]) -> MemoryScope {
    match flag_value(args, "--scope").as_deref() {
        Some("global") => MemoryScope::Global,
        Some("project") | None => default_project_scope(),
        Some(value) if value.starts_with("project:") => {
            MemoryScope::Project(value.trim_start_matches("project:").to_string())
        }
        Some(value) => MemoryScope::Project(value.to_string()),
    }
}

fn default_project_scope() -> MemoryScope {
    let project = std::env::current_dir()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "default".to_string());
    MemoryScope::Project(project)
}

fn parse_status(value: &str) -> anyhow::Result<KnowledgeStatus> {
    match value {
        "active" => Ok(KnowledgeStatus::Active),
        "draft" => Ok(KnowledgeStatus::Draft),
        "superseded" => Ok(KnowledgeStatus::Superseded),
        "archived" => Ok(KnowledgeStatus::Archived),
        other => anyhow::bail!("unknown status {other:?}"),
    }
}

fn parse_link_type(value: &str) -> anyhow::Result<KnowledgeLinkType> {
    match value {
        "relates_to" => Ok(KnowledgeLinkType::RelatesTo),
        "supersedes" => Ok(KnowledgeLinkType::Supersedes),
        "derived_from" => Ok(KnowledgeLinkType::DerivedFrom),
        "contradicts" => Ok(KnowledgeLinkType::Contradicts),
        "duplicates" => Ok(KnowledgeLinkType::Duplicates),
        other => anyhow::bail!("unknown link type {other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn positional_skips_flag_value_pairs() {
        let args = args(&[
            "update",
            "--title",
            "New title",
            "kn-1",
            "--status",
            "draft",
            "new body",
        ]);
        assert_eq!(positional(&args, 0).as_deref(), Some("update"));
        assert_eq!(positional(&args, 1).as_deref(), Some("kn-1"));
        assert_eq!(positional(&args, 2).as_deref(), Some("new body"));
    }

    #[test]
    fn positional_treats_bare_flags_as_valueless() {
        let args = args(&["search", "postgres", "--include-global"]);
        assert_eq!(positional(&args, 1).as_deref(), Some("postgres"));
        assert_eq!(positional(&args, 2), None);
    }

    #[test]
    fn flag_values_collects_repeated_flags() {
        let args = args(&["save", "--tag", "a", "--tag", "b"]);
        assert_eq!(flag_values(&args, "--tag"), vec!["a", "b"]);
    }

    #[test]
    fn scope_arg_parses_explicit_project() {
        let project = args(&["list", "--scope", "project:demo"]);
        assert_eq!(
            scope_arg(&project),
            MemoryScope::Project("demo".to_string())
        );
        let global = args(&["list", "--scope", "global"]);
        assert_eq!(scope_arg(&global), MemoryScope::Global);
    }
}
