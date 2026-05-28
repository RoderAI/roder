use roder_api::artifacts::{
    ArtifactGrepPage, ArtifactReadPage, ArtifactTailPage, ContextArtifact, ContextArtifactAccess,
    ContextArtifactId, CreateArtifactRequest,
};
use roder_api::events::ThreadId;
use sqlx_core::pool::Pool;
use sqlx_core::row::Row;
use sqlx_postgres::Postgres;
use time::OffsetDateTime;

const DEFAULT_PAGE_LINES: usize = 200;
const MAX_PAGE_LINES: usize = 200;

#[derive(Clone)]
pub struct PostgresArtifactStore {
    pub(crate) pool: Pool<Postgres>,
    pub(crate) tenant_id: String,
}

impl ContextArtifactAccess for PostgresArtifactStore {
    fn create_artifact(
        &self,
        request: CreateArtifactRequest<'_>,
    ) -> anyhow::Result<ContextArtifact> {
        let id = format!("artifact-{}", uuid::Uuid::new_v4());
        let bytes = request.bytes.to_vec();
        let artifact = ContextArtifact {
            id: id.clone(),
            kind: request.kind,
            thread_id: request.thread_id.clone(),
            turn_id: request.turn_id.clone(),
            byte_count: bytes.len() as u64,
            line_count: line_count_lossy(&bytes) as u64,
            source_tool_id: request.source_tool_id.map(ToOwned::to_owned),
            label: request.label.map(ToOwned::to_owned),
            store_path: format!(
                "postgres://tenant/{}/thread/{}/artifact/{id}",
                self.tenant_id, request.thread_id
            ),
            retention_expires_at: None,
            created_at: OffsetDateTime::now_utc(),
            roder_owned: true,
        };
        let pool = self.pool.clone();
        let tenant_id = self.tenant_id.clone();
        let artifact_for_db = artifact.clone();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                sqlx_core::query::query::<Postgres>(
                    "INSERT INTO roder_context_artifacts (tenant_id, thread_id, artifact_id, turn_id, metadata, body, created_at, updated_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$7)",
                )
                .bind(&tenant_id)
                .bind(&artifact_for_db.thread_id)
                .bind(&artifact_for_db.id)
                .bind(&artifact_for_db.turn_id)
                .bind(sqlx_core::types::Json(&artifact_for_db))
                .bind(bytes)
                .bind(artifact_for_db.created_at)
                .execute(&pool)
                .await
            })
        })?;
        Ok(artifact)
    }

    fn append_artifact(
        &self,
        thread_id: &ThreadId,
        artifact_id: &ContextArtifactId,
        bytes: &[u8],
    ) -> anyhow::Result<ContextArtifact> {
        let (mut artifact, mut body) = self.read_body_scoped(thread_id, artifact_id)?;
        body.extend_from_slice(bytes);
        artifact.byte_count = body.len() as u64;
        artifact.line_count = line_count_lossy(&body) as u64;
        let pool = self.pool.clone();
        let tenant_id = self.tenant_id.clone();
        let artifact_for_db = artifact.clone();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                sqlx_core::query::query::<Postgres>(
                    "UPDATE roder_context_artifacts SET metadata = $1, body = $2, updated_at = now() WHERE tenant_id = $3 AND thread_id = $4 AND artifact_id = $5",
                )
                .bind(sqlx_core::types::Json(&artifact_for_db))
                .bind(body)
                .bind(&tenant_id)
                .bind(&artifact_for_db.thread_id)
                .bind(&artifact_for_db.id)
                .execute(&pool)
                .await
            })
        })?;
        Ok(artifact)
    }

    fn list_artifacts(&self, thread_id: &ThreadId) -> anyhow::Result<Vec<ContextArtifact>> {
        let pool = self.pool.clone();
        let tenant_id = self.tenant_id.clone();
        let thread_id = thread_id.clone();
        let rows = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                sqlx_core::query::query::<Postgres>("SELECT metadata FROM roder_context_artifacts WHERE tenant_id = $1 AND thread_id = $2 ORDER BY created_at ASC, artifact_id ASC")
                    .bind(&tenant_id)
                    .bind(&thread_id)
                    .fetch_all(&pool)
                    .await
            })
        })?;
        rows.into_iter()
            .map(|row| {
                let json: sqlx_core::types::Json<ContextArtifact> = row.try_get("metadata")?;
                Ok(json.0)
            })
            .collect()
    }

    fn read_artifact(
        &self,
        thread_id: &ThreadId,
        artifact_id: &ContextArtifactId,
        start_line: usize,
        limit: usize,
    ) -> anyhow::Result<ArtifactReadPage> {
        let (artifact, body) = self.read_body_scoped(thread_id, artifact_id)?;
        let lines = numbered_lines(&body);
        let start_line = start_line.max(1);
        let limit = clamp_limit(Some(limit));
        let page = page_lines(&lines, start_line - 1, limit);
        Ok(ArtifactReadPage {
            artifact: artifact.descriptor(),
            text: page.text,
            start_line,
            limit,
            shown: page.shown,
            total_lines: page.total,
            next_start_line: page.next_offset.map(|offset| offset + 1),
            truncated: page.next_offset.is_some(),
        })
    }

    fn grep_artifact(
        &self,
        thread_id: &ThreadId,
        artifact_id: &ContextArtifactId,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> anyhow::Result<ArtifactGrepPage> {
        anyhow::ensure!(!query.is_empty(), "query is required");
        let (artifact, body) = self.read_body_scoped(thread_id, artifact_id)?;
        let matches = String::from_utf8_lossy(&body)
            .lines()
            .enumerate()
            .filter_map(|(index, line)| {
                line.contains(query)
                    .then(|| format!("{}: {}", index + 1, line))
            })
            .collect::<Vec<_>>();
        let limit = clamp_limit(Some(limit));
        let page = page_lines(&matches, offset, limit);
        Ok(ArtifactGrepPage {
            artifact: artifact.descriptor(),
            query: query.to_string(),
            text: page.text,
            offset,
            limit,
            shown: page.shown,
            total_matches: page.total,
            next_offset: page.next_offset,
            truncated: page.next_offset.is_some(),
        })
    }

    fn tail_artifact(
        &self,
        thread_id: &ThreadId,
        artifact_id: &ContextArtifactId,
        lines: usize,
    ) -> anyhow::Result<ArtifactTailPage> {
        let (artifact, body) = self.read_body_scoped(thread_id, artifact_id)?;
        let all_lines = numbered_lines(&body);
        let lines = clamp_limit(Some(lines));
        let total = all_lines.len();
        let start = total.saturating_sub(lines);
        let page = page_lines(&all_lines, start, lines);
        Ok(ArtifactTailPage {
            artifact: artifact.descriptor(),
            text: page.text,
            start_line: start + 1,
            lines,
            shown: page.shown,
            total_lines: page.total,
            truncated: start > 0,
        })
    }

    fn delete_artifact(
        &self,
        thread_id: &ThreadId,
        artifact_id: &ContextArtifactId,
    ) -> anyhow::Result<bool> {
        let artifact = self.get_scoped(thread_id, artifact_id)?;
        anyhow::ensure!(
            artifact.roder_owned,
            "refusing to delete non-Roder-owned artifact {}",
            artifact.id
        );
        let pool = self.pool.clone();
        let tenant_id = self.tenant_id.clone();
        let thread_id = thread_id.clone();
        let artifact_id = artifact_id.clone();
        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                sqlx_core::query::query::<Postgres>("DELETE FROM roder_context_artifacts WHERE tenant_id = $1 AND thread_id = $2 AND artifact_id = $3")
                    .bind(&tenant_id).bind(&thread_id).bind(&artifact_id).execute(&pool).await
            })
        })?;
        Ok(result.rows_affected() > 0)
    }
}

impl PostgresArtifactStore {
    fn get_scoped(
        &self,
        thread_id: &ThreadId,
        artifact_id: &ContextArtifactId,
    ) -> anyhow::Result<ContextArtifact> {
        Ok(self.read_body_scoped(thread_id, artifact_id)?.0)
    }

    fn read_body_scoped(
        &self,
        thread_id: &ThreadId,
        artifact_id: &ContextArtifactId,
    ) -> anyhow::Result<(ContextArtifact, Vec<u8>)> {
        let pool = self.pool.clone();
        let tenant_id = self.tenant_id.clone();
        let thread_id = thread_id.clone();
        let artifact_id = artifact_id.clone();
        let missing_artifact_id = artifact_id.clone();
        let row = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                sqlx_core::query::query::<Postgres>("SELECT metadata, body FROM roder_context_artifacts WHERE tenant_id = $1 AND thread_id = $2 AND artifact_id = $3")
                    .bind(&tenant_id).bind(&thread_id).bind(&artifact_id).fetch_optional(&pool).await
            })
        })?.ok_or_else(|| anyhow::anyhow!("unknown artifact {missing_artifact_id}"))?;
        let artifact: sqlx_core::types::Json<ContextArtifact> = row.try_get("metadata")?;
        let body: Vec<u8> = row.try_get("body")?;
        Ok((artifact.0, body))
    }
}

fn line_count_lossy(bytes: &[u8]) -> usize {
    let text = String::from_utf8_lossy(bytes);
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

fn numbered_lines(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .enumerate()
        .map(|(index, line)| format!("{}: {}", index + 1, line))
        .collect()
}

fn clamp_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_PAGE_LINES).clamp(1, MAX_PAGE_LINES)
}

struct LinePage {
    text: String,
    shown: usize,
    total: usize,
    next_offset: Option<usize>,
}

fn page_lines(lines: &[String], offset: usize, limit: usize) -> LinePage {
    let total = lines.len();
    let offset = offset.min(total);
    let end = offset.saturating_add(limit).min(total);
    let next_offset = (end < total).then_some(end);
    let mut text = lines[offset..end].join("\n");
    if let Some(next) = next_offset {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&format!(
            "[showing lines {}-{} of {total}; next_offset={next}]",
            offset + 1,
            end
        ));
    }
    LinePage {
        text,
        shown: end.saturating_sub(offset),
        total,
        next_offset,
    }
}
