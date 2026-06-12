//! Markdown-file-based `KnowledgeStore`.
//!
//! Layout under the base path:
//!
//! ```text
//! <base>/<scope-dir>/docs/<kind>/<slug>.md      # document head
//! <base>/<scope-dir>/revisions/<id>/<rev>.md    # immutable prior revisions
//! ```
//!
//! There is no index database: every operation scans and parses the markdown
//! files, so documents edited out-of-band are picked up on the next call.

use std::path::{Path, PathBuf};

use roder_api::extension::KnowledgeStoreId;
use roder_api::knowledge::{
    KnowledgeCitation, KnowledgeDocId, KnowledgeDocSummary, KnowledgeDocument, KnowledgeLink,
    KnowledgeLinkRequest, KnowledgeListQuery, KnowledgeQuery, KnowledgeRevisionInfo,
    KnowledgeSaveRequest, KnowledgeSearchResult, KnowledgeStatus, KnowledgeStore,
    KnowledgeStoreFactory, KnowledgeUpdateRequest,
};
use roder_api::memory::MemoryScope;
use time::OffsetDateTime;

use crate::document::{content_hash, parse_document, render_document, slugify};

pub const STORE_ID: &str = "markdown-knowledge";

/// Cap on document bodies. Knowledge documents are larger than memories but
/// must stay readable through paginated tools.
pub const MAX_BODY_BYTES: usize = 256 * 1024;
const MAX_TITLE_BYTES: usize = 512;

pub struct MarkdownKnowledgeStore {
    base_path: PathBuf,
}

impl MarkdownKnowledgeStore {
    pub fn new(base_path: PathBuf) -> Self {
        Self { base_path }
    }

    fn scope_dir(&self, scope: &MemoryScope) -> PathBuf {
        self.base_path.join(sanitize_component(&scope.stable_id()))
    }

    fn doc_path(&self, scope: &MemoryScope, kind: &str, slug: &str) -> PathBuf {
        self.scope_dir(scope)
            .join("docs")
            .join(sanitize_component(kind))
            .join(format!("{slug}.md"))
    }

    fn revisions_dir_for(&self, scope: &MemoryScope, id: &str) -> PathBuf {
        self.scope_dir(scope).join("revisions").join(id)
    }

    /// Scans every scope directory and parses each document. Parse failures
    /// surface as errors so corrupted documents are never silently dropped.
    fn scan_all(&self) -> anyhow::Result<Vec<(PathBuf, KnowledgeDocument)>> {
        let mut docs = Vec::new();
        let Ok(scopes) = std::fs::read_dir(&self.base_path) else {
            return Ok(docs);
        };
        for scope_entry in scopes.flatten() {
            if !scope_entry.path().is_dir() {
                continue;
            }
            let scope = parse_scope_dir(&scope_entry.file_name().to_string_lossy());
            let docs_dir = scope_entry.path().join("docs");
            let Ok(kinds) = std::fs::read_dir(&docs_dir) else {
                continue;
            };
            for kind_entry in kinds.flatten() {
                if !kind_entry.path().is_dir() {
                    continue;
                }
                let Ok(files) = std::fs::read_dir(kind_entry.path()) else {
                    continue;
                };
                for file in files.flatten() {
                    let path = file.path();
                    if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
                        continue;
                    }
                    let slug = path
                        .file_stem()
                        .map(|stem| stem.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let raw = std::fs::read_to_string(&path)?;
                    let doc = parse_document(&raw, scope.clone(), &slug).map_err(|error| {
                        anyhow::anyhow!("failed to parse {}: {error}", path.display())
                    })?;
                    docs.push((path, doc));
                }
            }
        }
        Ok(docs)
    }

    fn find_by_id(&self, id: &str) -> anyhow::Result<Option<(PathBuf, KnowledgeDocument)>> {
        Ok(self.scan_all()?.into_iter().find(|(_, doc)| doc.id == id))
    }

    fn unique_slug(&self, scope: &MemoryScope, kind: &str, title: &str) -> String {
        let base = slugify(title);
        let mut slug = base.clone();
        let mut counter = 2;
        while self.doc_path(scope, kind, &slug).exists() {
            slug = format!("{base}-{counter}");
            counter += 1;
        }
        slug
    }

    fn write_document(&self, path: &Path, doc: &KnowledgeDocument) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, render_document(doc)?)?;
        Ok(())
    }

    fn snapshot_revision(&self, doc: &KnowledgeDocument) -> anyhow::Result<()> {
        let dir = self.revisions_dir_for(&doc.scope, &doc.id);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(
            dir.join(format!("{}.md", doc.revision)),
            render_document(doc)?,
        )?;
        Ok(())
    }

    fn apply_update(
        &self,
        path: PathBuf,
        mut doc: KnowledgeDocument,
        mutate: impl FnOnce(&mut KnowledgeDocument),
    ) -> anyhow::Result<KnowledgeDocument> {
        self.snapshot_revision(&doc)?;
        mutate(&mut doc);
        doc.revision += 1;
        doc.updated_at = OffsetDateTime::now_utc();
        doc.content_hash = content_hash(&doc.body);
        self.write_document(&path, &doc)?;
        Ok(doc)
    }
}

#[async_trait::async_trait]
impl KnowledgeStore for MarkdownKnowledgeStore {
    fn id(&self) -> KnowledgeStoreId {
        STORE_ID.to_string()
    }

    async fn save(&self, request: KnowledgeSaveRequest) -> anyhow::Result<KnowledgeDocument> {
        validate_title(&request.title)?;
        validate_body(&request.body)?;
        let now = OffsetDateTime::now_utc();
        let slug = self.unique_slug(&request.scope, request.kind.as_str(), &request.title);
        let doc = KnowledgeDocument {
            id: format!("kn-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]),
            scope: request.scope,
            kind: request.kind,
            slug: slug.clone(),
            title: request.title,
            status: KnowledgeStatus::Active,
            source: request.source,
            tags: request.tags,
            links: Vec::new(),
            revision: 1,
            content_hash: content_hash(&request.body),
            body: request.body,
            created_at: now,
            updated_at: now,
        };
        let path = self.doc_path(&doc.scope, doc.kind.as_str(), &slug);
        self.write_document(&path, &doc)?;
        Ok(doc)
    }

    async fn get(&self, id: &KnowledgeDocId) -> anyhow::Result<Option<KnowledgeDocument>> {
        Ok(self.find_by_id(id)?.map(|(_, doc)| doc))
    }

    async fn get_revision(
        &self,
        id: &KnowledgeDocId,
        revision: u32,
    ) -> anyhow::Result<Option<KnowledgeDocument>> {
        let Some((_, head)) = self.find_by_id(id)? else {
            return Ok(None);
        };
        if head.revision == revision {
            return Ok(Some(head));
        }
        let path = self
            .revisions_dir_for(&head.scope, id)
            .join(format!("{revision}.md"));
        if !path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(&path)?;
        Ok(Some(parse_document(&raw, head.scope, &head.slug)?))
    }

    async fn list(&self, query: KnowledgeListQuery) -> anyhow::Result<Vec<KnowledgeDocSummary>> {
        let mut docs = self
            .scan_all()?
            .into_iter()
            .map(|(_, doc)| doc)
            .filter(|doc| matches_scope(doc, query.scope.as_ref(), false))
            .filter(|doc| {
                query
                    .kind
                    .as_ref()
                    .is_none_or(|kind| doc.kind.as_str() == kind.as_str())
            })
            .filter(|doc| {
                query
                    .tag
                    .as_ref()
                    .is_none_or(|tag| doc.tags.iter().any(|t| t.eq_ignore_ascii_case(tag)))
            })
            .filter(|doc| match query.status {
                Some(status) => doc.status == status,
                None => query.include_archived || doc.status != KnowledgeStatus::Archived,
            })
            .collect::<Vec<_>>();
        docs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        docs.truncate(query.limit.max(1));
        Ok(docs.iter().map(KnowledgeDocument::summary).collect())
    }

    async fn search(&self, query: KnowledgeQuery) -> anyhow::Result<Vec<KnowledgeSearchResult>> {
        let mut results = self
            .scan_all()?
            .into_iter()
            .map(|(_, doc)| doc)
            .filter(|doc| doc.status != KnowledgeStatus::Archived)
            .filter(|doc| matches_scope(doc, query.scope.as_ref(), query.include_global))
            .filter(|doc| {
                query
                    .kind
                    .as_ref()
                    .is_none_or(|kind| doc.kind.as_str() == kind.as_str())
            })
            .filter_map(|doc| {
                let haystack =
                    format!("{}\n{}\n{}", doc.title, doc.tags.join(" "), doc.body);
                let score = lexical_score(&query.text, &haystack);
                if score <= 0.0 {
                    return None;
                }
                let snippet = snippet_around_match(&doc.body, &query.text)
                    .unwrap_or_else(|| bounded_snippet(&doc.body));
                let citation = KnowledgeCitation {
                    doc_id: doc.id.clone(),
                    scope_id: doc.scope.stable_id(),
                    title: doc.title.clone(),
                    snippet: snippet.clone(),
                    score_millis: (score * 1000.0) as u32,
                };
                Some(KnowledgeSearchResult {
                    document: doc.summary(),
                    score,
                    snippet,
                    citation,
                })
            })
            .collect::<Vec<_>>();
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.document.updated_at.cmp(&a.document.updated_at))
        });
        results.truncate(query.limit.max(1));
        Ok(results)
    }

    async fn update(&self, request: KnowledgeUpdateRequest) -> anyhow::Result<KnowledgeDocument> {
        if let Some(title) = &request.title {
            validate_title(title)?;
        }
        if let Some(body) = &request.body {
            validate_body(body)?;
        }
        let (path, doc) = self
            .find_by_id(&request.id)?
            .ok_or_else(|| anyhow::anyhow!("knowledge document not found: {}", request.id))?;
        self.apply_update(path, doc, |doc| {
            if let Some(title) = request.title {
                doc.title = title;
            }
            if let Some(body) = request.body {
                doc.body = body;
            }
            if let Some(status) = request.status {
                doc.status = status;
            }
            if let Some(tags) = request.tags {
                doc.tags = tags;
            }
            doc.source = request.source;
        })
    }

    async fn archive(&self, id: &KnowledgeDocId) -> anyhow::Result<bool> {
        let Some((path, doc)) = self.find_by_id(id)? else {
            return Ok(false);
        };
        if doc.status == KnowledgeStatus::Archived {
            return Ok(false);
        }
        self.apply_update(path, doc, |doc| {
            doc.status = KnowledgeStatus::Archived;
        })?;
        Ok(true)
    }

    async fn set_link(
        &self,
        request: KnowledgeLinkRequest,
    ) -> anyhow::Result<KnowledgeDocument> {
        let (path, doc) = self
            .find_by_id(&request.from)?
            .ok_or_else(|| anyhow::anyhow!("knowledge document not found: {}", request.from))?;
        if !request.remove && self.find_by_id(&request.to)?.is_none() {
            anyhow::bail!("knowledge link target not found: {}", request.to);
        }
        self.apply_update(path, doc, |doc| {
            doc.links
                .retain(|link| !(link.link_type == request.link_type && link.to == request.to));
            if !request.remove {
                doc.links.push(KnowledgeLink {
                    link_type: request.link_type,
                    to: request.to,
                });
            }
        })
    }

    async fn revisions(
        &self,
        id: &KnowledgeDocId,
    ) -> anyhow::Result<Vec<KnowledgeRevisionInfo>> {
        let Some((_, head)) = self.find_by_id(id)? else {
            return Ok(Vec::new());
        };
        let mut revisions = vec![KnowledgeRevisionInfo {
            revision: head.revision,
            content_hash: head.content_hash.clone(),
            created_at: head.updated_at,
        }];
        let dir = self.revisions_dir_for(&head.scope, id);
        if let Ok(files) = std::fs::read_dir(&dir) {
            for file in files.flatten() {
                let path = file.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
                    continue;
                }
                let raw = std::fs::read_to_string(&path)?;
                let doc = parse_document(&raw, head.scope.clone(), &head.slug)?;
                revisions.push(KnowledgeRevisionInfo {
                    revision: doc.revision,
                    content_hash: doc.content_hash,
                    created_at: doc.updated_at,
                });
            }
        }
        revisions.sort_by(|a, b| b.revision.cmp(&a.revision));
        revisions.dedup_by_key(|info| info.revision);
        Ok(revisions)
    }
}

pub struct MarkdownKnowledgeStoreFactory {
    base_path: PathBuf,
}

impl MarkdownKnowledgeStoreFactory {
    pub fn new(base_path: PathBuf) -> Self {
        Self { base_path }
    }
}

impl KnowledgeStoreFactory for MarkdownKnowledgeStoreFactory {
    fn id(&self) -> KnowledgeStoreId {
        STORE_ID.to_string()
    }

    fn create(&self) -> std::sync::Arc<dyn KnowledgeStore> {
        std::sync::Arc::new(MarkdownKnowledgeStore::new(self.base_path.clone()))
    }
}

fn validate_title(title: &str) -> anyhow::Result<()> {
    if title.trim().is_empty() {
        anyhow::bail!("knowledge title must not be empty");
    }
    if title.len() > MAX_TITLE_BYTES {
        anyhow::bail!("knowledge title is {} bytes; the limit is {MAX_TITLE_BYTES}", title.len());
    }
    if title.contains('\n') {
        anyhow::bail!("knowledge title must be a single line");
    }
    Ok(())
}

fn validate_body(body: &str) -> anyhow::Result<()> {
    if body.len() > MAX_BODY_BYTES {
        anyhow::bail!(
            "knowledge body is {} bytes; the limit is {MAX_BODY_BYTES} bytes — split the document",
            body.len()
        );
    }
    Ok(())
}

fn matches_scope(
    doc: &KnowledgeDocument,
    scope: Option<&MemoryScope>,
    include_global: bool,
) -> bool {
    match scope {
        None => true,
        Some(scope) => {
            doc.scope == *scope || (include_global && doc.scope == MemoryScope::Global)
        }
    }
}

fn lexical_score(query: &str, text: &str) -> f32 {
    let query = query.to_lowercase();
    let text = text.to_lowercase();
    let terms = query.split_whitespace().collect::<Vec<_>>();
    if terms.is_empty() {
        return 1.0;
    }
    terms.iter().filter(|term| text.contains(**term)).count() as f32 / terms.len() as f32
}

const SNIPPET_CONTEXT_BYTES: usize = 120;

fn snippet_around_match(body: &str, query: &str) -> Option<String> {
    let lower_body = body.to_lowercase();
    let position = query
        .to_lowercase()
        .split_whitespace()
        .find_map(|term| lower_body.find(term))?;
    let mut start = position.saturating_sub(SNIPPET_CONTEXT_BYTES);
    while start > 0 && !body.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (position + SNIPPET_CONTEXT_BYTES).min(body.len());
    while end < body.len() && !body.is_char_boundary(end) {
        end += 1;
    }
    let mut snippet = body[start..end].split_whitespace().collect::<Vec<_>>().join(" ");
    if start > 0 {
        snippet = format!("...{snippet}");
    }
    if end < body.len() {
        snippet.push_str("...");
    }
    Some(snippet)
}

fn bounded_snippet(body: &str) -> String {
    const MAX: usize = 180;
    if body.chars().count() <= MAX {
        body.to_string()
    } else {
        let mut out = body.chars().take(MAX).collect::<String>();
        out.push_str("...");
        out
    }
}

fn sanitize_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    if out.is_empty() { "default".to_string() } else { out }
}

fn parse_scope_dir(dir: &str) -> MemoryScope {
    match dir {
        "global" => MemoryScope::Global,
        value if value.starts_with("project-") => {
            MemoryScope::Project(value.trim_start_matches("project-").to_string())
        }
        value if value.starts_with("workspace-") => {
            MemoryScope::Workspace(value.trim_start_matches("workspace-").to_string())
        }
        value if value.starts_with("user-") => {
            MemoryScope::User(value.trim_start_matches("user-").to_string())
        }
        value if value.starts_with("thread-") => {
            MemoryScope::Thread(value.trim_start_matches("thread-").to_string())
        }
        value => MemoryScope::Project(value.to_string()),
    }
}
