//! `/knowledge` slash command: browse, search, and read the project
//! knowledge base from the TUI (roadmap phase 93).

use roder_api::memory::MemoryScope;
use roder_app_server::AppClient;
use roder_protocol::{
    JsonRpcRequest, KnowledgeListParams, KnowledgeListResult, KnowledgeReadParams,
    KnowledgeReadResult, KnowledgeSearchParams, KnowledgeSearchResults,
};

use super::{TuiApp, decode_response};

const USAGE: &str =
    "usage: /knowledge [list [kind]|search <text>|read <id>] — manage documents with knowledge_* tools or `roder knowledge`";

/// Cap on the body text shown inline for `/knowledge read`.
const READ_PREVIEW_BYTES: usize = 4096;

impl<C> TuiApp<C>
where
    C: AppClient,
{
    pub(super) async fn run_knowledge_slash_command(&mut self, args: &str) {
        let args = args.trim();
        let (action, rest) = match args.split_once(char::is_whitespace) {
            Some((action, rest)) => (action, rest.trim()),
            None => (args, ""),
        };
        match action {
            "" | "list" => self.knowledge_list(rest).await,
            "search" if !rest.is_empty() => self.knowledge_search(rest).await,
            "read" if !rest.is_empty() => self.knowledge_read(rest).await,
            _ => {
                self.timeline.push_system(USAGE);
            }
        }
        self.push_event(format!("slash command: /knowledge {args}").trim().to_string());
    }

    async fn knowledge_list(&mut self, kind: &str) {
        let params = KnowledgeListParams {
            scope: Some(default_project_scope()),
            kind: (!kind.is_empty())
                .then(|| roder_api::knowledge::KnowledgeKind::parse(kind)),
            tag: None,
            status: None,
            include_archived: false,
            limit: Some(50),
        };
        match request::<_, KnowledgeListResult>(&self.client, "knowledge/list", params).await {
            Ok(result) if result.documents.is_empty() => {
                self.timeline.push_system(
                    "No knowledge documents yet. Save one with the knowledge_save tool or `roder knowledge save`.",
                );
            }
            Ok(result) => {
                let lines = result
                    .documents
                    .iter()
                    .map(|doc| {
                        format!(
                            "{}  {:<11}  {} (rev {})",
                            doc.id, doc.kind, doc.title, doc.revision
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                self.timeline.push_system(format!(
                    "Knowledge documents ({}):\n{lines}",
                    result.documents.len()
                ));
            }
            Err(err) => self.record_error(format!("knowledge/list failed: {err}")),
        }
    }

    async fn knowledge_search(&mut self, text: &str) {
        let params = KnowledgeSearchParams {
            scope: Some(default_project_scope()),
            text: text.to_string(),
            kind: None,
            limit: Some(8),
            include_global: true,
        };
        match request::<_, KnowledgeSearchResults>(&self.client, "knowledge/search", params).await
        {
            Ok(result) if result.results.is_empty() => {
                self.timeline
                    .push_system(format!("No knowledge documents match {text:?}."));
            }
            Ok(result) => {
                let lines = result
                    .results
                    .iter()
                    .map(|matched| {
                        format!(
                            "{:.2}  {}  {}\n      {}",
                            matched.score,
                            matched.document.id,
                            matched.document.title,
                            matched.snippet
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                self.timeline
                    .push_system(format!("Knowledge search {text:?}:\n{lines}"));
            }
            Err(err) => self.record_error(format!("knowledge/search failed: {err}")),
        }
    }

    async fn knowledge_read(&mut self, doc_id: &str) {
        let params = KnowledgeReadParams {
            doc_id: doc_id.to_string(),
            revision: None,
        };
        match request::<_, KnowledgeReadResult>(&self.client, "knowledge/read", params).await {
            Ok(result) => match result.document {
                Some(doc) => {
                    let mut body = doc.body;
                    if body.len() > READ_PREVIEW_BYTES {
                        let mut end = READ_PREVIEW_BYTES;
                        while end > 0 && !body.is_char_boundary(end) {
                            end -= 1;
                        }
                        body.truncate(end);
                        body.push_str("\n... [truncated; see `roder knowledge read` for the full document]");
                    }
                    self.timeline.push_system(format!(
                        "# {} ({}, {}, rev {})\n\n{}",
                        doc.title,
                        doc.kind,
                        doc.status.as_str(),
                        doc.revision,
                        body
                    ));
                }
                None => {
                    self.timeline
                        .push_system(format!("Knowledge document not found: {doc_id}"));
                }
            },
            Err(err) => self.record_error(format!("knowledge/read failed: {err}")),
        }
    }
}

async fn request<P: serde::Serialize, T: serde::de::DeserializeOwned>(
    client: &impl AppClient,
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
    decode_response(res)
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
