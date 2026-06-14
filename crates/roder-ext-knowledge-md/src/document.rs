//! Canonical markdown document format: YAML front matter + body.
//!
//! Scope is implied by the directory a document lives in, not the front
//! matter, so scope folders stay self-contained and relocatable.

use roder_api::knowledge::{
    KnowledgeDocument, KnowledgeKind, KnowledgeLink, KnowledgeLinkType, KnowledgeSource,
    KnowledgeStatus,
};
use roder_api::memory::MemoryScope;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

const FRONT_MATTER_FENCE: &str = "---";

#[derive(Debug, Serialize, Deserialize)]
struct FrontMatter {
    id: String,
    kind: String,
    title: String,
    status: String,
    source: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    links: Vec<LinkEntry>,
    revision: u32,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct LinkEntry {
    #[serde(rename = "type")]
    link_type: String,
    to: String,
}

pub fn content_hash(body: &str) -> String {
    let digest = Sha256::digest(body.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

pub fn render_document(doc: &KnowledgeDocument) -> anyhow::Result<String> {
    let front = FrontMatter {
        id: doc.id.clone(),
        kind: doc.kind.as_str().to_string(),
        title: doc.title.clone(),
        status: doc.status.as_str().to_string(),
        source: source_to_str(doc.source).to_string(),
        tags: doc.tags.clone(),
        links: doc
            .links
            .iter()
            .map(|link| LinkEntry {
                link_type: link.link_type.as_str().to_string(),
                to: link.to.clone(),
            })
            .collect(),
        revision: doc.revision,
        created_at: doc.created_at.format(&Rfc3339)?,
        updated_at: doc.updated_at.format(&Rfc3339)?,
    };
    let yaml = serde_yaml::to_string(&front)?;
    Ok(format!(
        "{FRONT_MATTER_FENCE}\n{yaml}{FRONT_MATTER_FENCE}\n\n{}",
        doc.body
    ))
}

/// Parses a markdown file into a document. `scope` and `slug` come from the
/// file location; everything else comes from front matter and body. The
/// content hash is always derived from the body so out-of-band edits are
/// reflected on next load.
pub fn parse_document(
    raw: &str,
    scope: MemoryScope,
    slug: &str,
) -> anyhow::Result<KnowledgeDocument> {
    let rest = raw
        .strip_prefix(FRONT_MATTER_FENCE)
        .and_then(|rest| rest.strip_prefix('\n'))
        .ok_or_else(|| anyhow::anyhow!("knowledge document is missing front matter"))?;
    let (front_raw, body) = rest
        .split_once("\n---")
        .ok_or_else(|| anyhow::anyhow!("knowledge document front matter is unterminated"))?;
    // The renderer writes `---\n\n` between front matter and body; strip the
    // fence-line newline plus the single separator blank line.
    let body = body.strip_prefix('\n').unwrap_or(body);
    let body = body.strip_prefix('\n').unwrap_or(body).to_string();
    let front: FrontMatter = serde_yaml::from_str(front_raw)?;

    let links = front
        .links
        .into_iter()
        .map(|entry| {
            Ok(KnowledgeLink {
                link_type: parse_link_type(&entry.link_type)?,
                to: entry.to,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    Ok(KnowledgeDocument {
        id: front.id,
        scope,
        kind: KnowledgeKind::parse(&front.kind),
        slug: slug.to_string(),
        title: front.title,
        status: parse_status(&front.status)?,
        source: parse_source(&front.source)?,
        tags: front.tags,
        links,
        revision: front.revision,
        content_hash: content_hash(&body),
        body,
        created_at: OffsetDateTime::parse(&front.created_at, &Rfc3339)?,
        updated_at: OffsetDateTime::parse(&front.updated_at, &Rfc3339)?,
    })
}

pub fn slugify(title: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = true;
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "untitled".to_string()
    } else {
        slug.chars().take(64).collect()
    }
}

fn source_to_str(source: KnowledgeSource) -> &'static str {
    match source {
        KnowledgeSource::User => "user",
        KnowledgeSource::Agent => "agent",
        KnowledgeSource::Reconciler => "reconciler",
        KnowledgeSource::Import => "import",
    }
}

fn parse_source(value: &str) -> anyhow::Result<KnowledgeSource> {
    match value {
        "user" => Ok(KnowledgeSource::User),
        "agent" => Ok(KnowledgeSource::Agent),
        "reconciler" => Ok(KnowledgeSource::Reconciler),
        "import" => Ok(KnowledgeSource::Import),
        other => anyhow::bail!("unknown knowledge source {other:?}"),
    }
}

fn parse_status(value: &str) -> anyhow::Result<KnowledgeStatus> {
    match value {
        "active" => Ok(KnowledgeStatus::Active),
        "draft" => Ok(KnowledgeStatus::Draft),
        "superseded" => Ok(KnowledgeStatus::Superseded),
        "archived" => Ok(KnowledgeStatus::Archived),
        other => anyhow::bail!("unknown knowledge status {other:?}"),
    }
}

fn parse_link_type(value: &str) -> anyhow::Result<KnowledgeLinkType> {
    match value {
        "relates_to" => Ok(KnowledgeLinkType::RelatesTo),
        "supersedes" => Ok(KnowledgeLinkType::Supersedes),
        "derived_from" => Ok(KnowledgeLinkType::DerivedFrom),
        "contradicts" => Ok(KnowledgeLinkType::Contradicts),
        "duplicates" => Ok(KnowledgeLinkType::Duplicates),
        other => anyhow::bail!("unknown knowledge link type {other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_doc() -> KnowledgeDocument {
        let now = OffsetDateTime::UNIX_EPOCH;
        KnowledgeDocument {
            id: "kn-abc12345".to_string(),
            scope: MemoryScope::Project("demo".to_string()),
            kind: KnowledgeKind::Decision,
            slug: "use-markdown".to_string(),
            title: "Use markdown: a decision".to_string(),
            status: KnowledgeStatus::Active,
            source: KnowledgeSource::User,
            tags: vec!["storage".to_string(), "format".to_string()],
            links: vec![KnowledgeLink {
                link_type: KnowledgeLinkType::Supersedes,
                to: "kn-old00000".to_string(),
            }],
            revision: 2,
            content_hash: content_hash("We picked markdown.\n\n---\n\nBecause files."),
            body: "We picked markdown.\n\n---\n\nBecause files.".to_string(),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn document_round_trips_through_markdown() {
        let doc = sample_doc();
        let raw = render_document(&doc).unwrap();
        let parsed =
            parse_document(&raw, MemoryScope::Project("demo".to_string()), &doc.slug).unwrap();

        assert_eq!(parsed, doc);
    }

    #[test]
    fn body_keeps_horizontal_rules_intact() {
        let doc = sample_doc();
        let raw = render_document(&doc).unwrap();
        let parsed =
            parse_document(&raw, MemoryScope::Project("demo".to_string()), &doc.slug).unwrap();

        assert!(parsed.body.contains("\n---\n"));
    }

    #[test]
    fn parse_rejects_files_without_front_matter() {
        let error = parse_document("just a note", MemoryScope::Global, "note").unwrap_err();
        assert!(error.to_string().contains("front matter"));
    }

    #[test]
    fn slugify_normalizes_titles() {
        assert_eq!(
            slugify("Use SQLite for analytics!"),
            "use-sqlite-for-analytics"
        );
        assert_eq!(slugify("  ---  "), "untitled");
        assert_eq!(slugify("Üñïcode Heavy"), "code-heavy");
    }

    #[test]
    fn content_hash_tracks_body_changes() {
        assert_ne!(content_hash("a"), content_hash("b"));
        assert_eq!(content_hash("a"), content_hash("a"));
    }
}
