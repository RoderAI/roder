//! Cursor model listing.
//!
//! Roder ships a curated static catalog of Cursor models (known-good ids that
//! the `agent.v1.AgentService/Run` path accepts, with hand-tuned context
//! windows and reasoning options). On top of that, `list_models` refreshes the
//! set from Cursor's live model picker RPC and caches the merged result on disk
//! (same pattern as the OpenAI/OpenCode/Poolside providers) so newly released
//! Cursor models appear without a Roder release.
//!
//! Endpoint (reverse-engineered from the Cursor app bundle + verified live):
//!   POST {backend}/aiserver.v1.AiService/AvailableModels
//!   content-type: application/json   body: {}   bearer: <access token>
//! Response: `{ "models": [ { serverModelName, clientDisplayName,
//!   supportsAgent, supportsImages, supportsThinking, tooltipData{ markdownContent } } ] }`.
//!
//! IMPORTANT: the picker returns effort/fast variant ids (e.g.
//! `claude-opus-4-8-high`, `composer-2.5-fast`) which DIVERGE from the bare ids
//! the Run path accepts (`claude-opus-4-8`). We therefore reduce picker ids to
//! their base form and merge them into the curated catalog rather than trusting
//! them verbatim: curated entries keep their metadata; genuinely new base ids
//! are appended with metadata derived from the picker payload.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use roder_api::catalog::{PROVIDER_CURSOR, models_for_provider};
use roder_api::inference::ModelDescriptor;
use serde::{Deserialize, Serialize};

const DEFAULT_MODELS_CACHE_TTL: Duration = Duration::from_secs(6 * 60 * 60);
const AVAILABLE_MODELS_PATH: &str = "/aiserver.v1.AiService/AvailableModels";

/// The curated static catalog — always the authoritative base set and the
/// fallback when the live call is unavailable.
pub fn fallback_models() -> Vec<ModelDescriptor> {
    models_for_provider(PROVIDER_CURSOR, false)
}

// ===== Live discovery =====

#[derive(Debug, Deserialize)]
struct AvailableModelsResponse {
    #[serde(default)]
    models: Vec<AvailableModel>,
}

#[derive(Debug, Deserialize)]
struct AvailableModel {
    #[serde(default, rename = "serverModelName")]
    server_model_name: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default, rename = "clientDisplayName")]
    client_display_name: Option<String>,
    #[serde(default, rename = "supportsAgent")]
    supports_agent: bool,
    #[serde(default, rename = "tooltipData")]
    tooltip_data: Option<TooltipData>,
}

#[derive(Debug, Deserialize)]
struct TooltipData {
    #[serde(default, rename = "markdownContent")]
    markdown_content: String,
}

/// Fetch the agent-capable model set from Cursor's picker RPC and merge it into
/// the curated catalog. Errors (auth/network/HTTP) propagate so the caller can
/// fall back to the static catalog.
pub async fn discover_models(
    backend_base_url: String,
    access_token: String,
    client_version: String,
) -> anyhow::Result<Vec<ModelDescriptor>> {
    let url = format!(
        "{}{}",
        backend_base_url.trim_end_matches('/'),
        AVAILABLE_MODELS_PATH
    );
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    let response = client
        .post(url)
        .bearer_auth(&access_token)
        .header("connect-protocol-version", "1")
        .header("content-type", "application/json")
        .header("x-cursor-client-type", "cli")
        .header("x-cursor-client-version", client_version)
        .header("x-ghost-mode", "true")
        .body("{}")
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("Cursor AvailableModels failed with HTTP {status}");
    }
    let payload: AvailableModelsResponse = response.json().await?;
    Ok(merge_live_into_catalog(payload.models))
}

/// Merge agent-capable picker models into the curated catalog: curated entries
/// are preserved verbatim (curated ids + rich metadata), then any new base id
/// not already present is appended with derived metadata.
fn merge_live_into_catalog(live: Vec<AvailableModel>) -> Vec<ModelDescriptor> {
    let mut models = fallback_models();
    let mut known: std::collections::HashSet<String> =
        models.iter().map(|model| model.id.clone()).collect();

    // Aggregate live variants by base id, keeping the largest context window
    // seen and a cleaned display name.
    let mut discovered: BTreeMap<String, (String, Option<u32>)> = BTreeMap::new();
    for model in live {
        if !model.supports_agent {
            continue;
        }
        let Some(raw_id) = model
            .server_model_name
            .clone()
            .or_else(|| model.name.clone())
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
        else {
            continue;
        };
        let base = base_model_id(&raw_id);
        if !is_appendable_base_id(&base) {
            continue;
        }
        let display = clean_display_name(
            model
                .client_display_name
                .as_deref()
                .unwrap_or(base.as_str()),
        );
        let context_window = model
            .tooltip_data
            .as_ref()
            .and_then(|tooltip| context_window_from_text(&tooltip.markdown_content));
        let entry = discovered
            .entry(base)
            .or_insert_with(|| (display.clone(), context_window));
        if context_window.unwrap_or(0) > entry.1.unwrap_or(0) {
            entry.1 = context_window;
        }
    }

    for (id, (name, context_window)) in discovered {
        if known.contains(&id) {
            continue;
        }
        known.insert(id.clone());
        models.push(ModelDescriptor {
            id,
            name,
            context_window,
            default_reasoning: None,
            supported_reasoning: Vec::new(),
        });
    }
    models
}

/// Reduce a Cursor picker id to the bare model id the Run path accepts by
/// stripping the trailing effort / thinking / fast modifiers Cursor appends
/// (e.g. `claude-opus-4-8-thinking-high-fast` -> `claude-opus-4-8`).
fn base_model_id(id: &str) -> String {
    let mut s = id;
    loop {
        let trimmed = s
            .strip_suffix("-fast")
            .or_else(|| s.strip_suffix("-thinking"))
            .or_else(|| strip_effort_suffix(s))
            .unwrap_or(s);
        if trimmed == s {
            break;
        }
        s = trimmed;
    }
    s.to_string()
}

fn strip_effort_suffix(s: &str) -> Option<&str> {
    for effort in [
        "-extra-high",
        "-xhigh",
        "-medium",
        "-high",
        "-low",
        "-none",
        "-max",
    ] {
        if let Some(rest) = s.strip_suffix(effort) {
            return Some(rest);
        }
    }
    None
}

/// Conservatively decide whether a discovered base id should be surfaced. We
/// skip meta ids (`auto`/`default`), namespaced/provider-prefixed ids, and
/// anything that does not look like a normal model slug, since those are not
/// known to work on the Run path.
fn is_appendable_base_id(base: &str) -> bool {
    if base.is_empty() || base.contains('/') {
        return false;
    }
    if matches!(base, "auto" | "default") {
        return false;
    }
    base.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

/// Strip trailing effort/mode words Cursor adds to display names so the base
/// model reads cleanly (e.g. "Opus 4.8 Extra High Fast" -> "Opus 4.8").
fn clean_display_name(name: &str) -> String {
    let drop = [
        "Fast", "Thinking", "Max", "High", "Extra", "Medium", "Low", "None",
    ];
    let mut words: Vec<&str> = name.split_whitespace().collect();
    while let Some(last) = words.last() {
        if drop.contains(last) {
            words.pop();
        } else {
            break;
        }
    }
    let cleaned = words.join(" ");
    if cleaned.is_empty() {
        name.trim().to_string()
    } else {
        cleaned
    }
}

/// Parse a context window like `200k` / `1M` from Cursor's tooltip markdown
/// (e.g. "...<br />200k context window<br />..."). Returns the token count.
fn context_window_from_text(text: &str) -> Option<u32> {
    let pos = text.find("context window")?;
    // Walk back from "context window" over an optional space, a `k`/`m` unit,
    // and the contiguous digits before it.
    let prefix = text[..pos].trim_end();
    let (unit_idx, mult) = prefix.char_indices().rev().find_map(|(idx, c)| match c {
        'k' | 'K' => Some((idx, 1_000u64)),
        'm' | 'M' => Some((idx, 1_000_000u64)),
        _ => None,
    })?;
    let digits: String = prefix[..unit_idx]
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    let n: u64 = digits.parse().ok()?;
    Some(n.saturating_mul(mult).min(u64::from(u32::MAX)) as u32)
}

// ===== On-disk cache (mirrors the other providers) =====

#[derive(Debug, Default, Serialize, Deserialize)]
struct ModelsCacheFile {
    #[serde(default)]
    providers: BTreeMap<String, CachedProviderModels>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CachedProviderModels {
    fetched_at: u64,
    backend_base_url: String,
    pub(crate) models: Vec<ModelDescriptor>,
}

impl CachedProviderModels {
    pub(crate) fn is_stale(&self, ttl: Duration) -> bool {
        ttl.is_zero()
            || now_unix_secs()
                .saturating_sub(self.fetched_at)
                .ge(&ttl.as_secs())
    }
}

pub(crate) fn cached_models(backend_base_url: &str) -> anyhow::Result<CachedProviderModels> {
    let cache: ModelsCacheFile = serde_json::from_str(&fs::read_to_string(cache_path())?)?;
    cache
        .providers
        .get(PROVIDER_CURSOR)
        .filter(|entry| {
            entry.backend_base_url.trim_end_matches('/') == backend_base_url.trim_end_matches('/')
        })
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("no cached models for cursor"))
}

pub(crate) fn save_cached_models(
    backend_base_url: &str,
    models: &[ModelDescriptor],
) -> anyhow::Result<()> {
    let path = cache_path();
    let mut cache = fs::read_to_string(&path)
        .ok()
        .and_then(|body| serde_json::from_str::<ModelsCacheFile>(&body).ok())
        .unwrap_or_default();
    cache.providers.insert(
        PROVIDER_CURSOR.to_string(),
        CachedProviderModels {
            fetched_at: now_unix_secs(),
            backend_base_url: backend_base_url.trim_end_matches('/').to_string(),
            models: models.to_vec(),
        },
    );
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(&cache)?)?;
    Ok(())
}

pub(crate) fn cache_ttl() -> Duration {
    env_nonempty("RODER_MODELS_CACHE_TTL_SECONDS")
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_MODELS_CACHE_TTL)
}

pub(crate) fn force_refresh_requested() -> bool {
    env_nonempty("RODER_MODELS_REFRESH")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

fn cache_path() -> PathBuf {
    if let Some(path) = env_nonempty("RODER_MODELS_CACHE_PATH") {
        return PathBuf::from(path);
    }
    roder_data_dir().join("models-cache.json")
}

fn roder_data_dir() -> PathBuf {
    std::env::var_os("RODER_DATA_DIR")
        .or_else(|| std::env::var_os("RODER_CONFIG_DIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".roder")
        })
}

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_model_id_strips_effort_thinking_and_fast_suffixes() {
        assert_eq!(base_model_id("claude-opus-4-8"), "claude-opus-4-8");
        assert_eq!(base_model_id("claude-opus-4-8-high"), "claude-opus-4-8");
        assert_eq!(
            base_model_id("claude-opus-4-8-thinking-high-fast"),
            "claude-opus-4-8"
        );
        assert_eq!(base_model_id("composer-2.5-fast"), "composer-2.5");
        assert_eq!(base_model_id("gpt-5.5-extra-high"), "gpt-5.5");
        assert_eq!(base_model_id("gpt-5.3-codex-xhigh-fast"), "gpt-5.3-codex");
    }

    #[test]
    fn appendable_filter_rejects_meta_and_namespaced_ids() {
        assert!(is_appendable_base_id("composer-2.5"));
        assert!(is_appendable_base_id("gpt-5.5"));
        assert!(!is_appendable_base_id("auto"));
        assert!(!is_appendable_base_id("default"));
        assert!(!is_appendable_base_id(
            "accounts/fireworks/models/kimi-k2p5"
        ));
        assert!(!is_appendable_base_id(""));
    }

    #[test]
    fn clean_display_name_drops_trailing_effort_words() {
        assert_eq!(clean_display_name("Opus 4.8 Extra High Fast"), "Opus 4.8");
        assert_eq!(clean_display_name("Composer 2.5"), "Composer 2.5");
        assert_eq!(clean_display_name("GPT-5.5 Low"), "GPT-5.5");
    }

    #[test]
    fn context_window_parses_k_and_m() {
        assert_eq!(
            context_window_from_text("blah 200k context window blah"),
            Some(200_000)
        );
        assert_eq!(
            context_window_from_text("Fable 5 ... 1M context window ..."),
            Some(1_000_000)
        );
        assert_eq!(context_window_from_text("no window here"), None);
    }

    #[test]
    fn merge_keeps_curated_models_and_appends_new_base_ids() {
        let live = vec![
            // Already curated -> must NOT duplicate.
            AvailableModel {
                server_model_name: Some("claude-opus-4-8-high".to_string()),
                name: None,
                client_display_name: Some("Opus 4.8".to_string()),
                supports_agent: true,
                tooltip_data: Some(TooltipData {
                    markdown_content: "300k context window".to_string(),
                }),
            },
            // Brand new base id -> appended.
            AvailableModel {
                server_model_name: Some("gpt-9-turbo-high".to_string()),
                name: None,
                client_display_name: Some("GPT-9 Turbo High".to_string()),
                supports_agent: true,
                tooltip_data: Some(TooltipData {
                    markdown_content: "**GPT-9**<br />500k context window".to_string(),
                }),
            },
            // Non-agent -> ignored.
            AvailableModel {
                server_model_name: Some("embedding-x".to_string()),
                name: None,
                client_display_name: Some("Embedding".to_string()),
                supports_agent: false,
                tooltip_data: None,
            },
        ];
        let merged = merge_live_into_catalog(live);
        let ids: Vec<&str> = merged.iter().map(|m| m.id.as_str()).collect();
        assert!(ids.contains(&"claude-opus-4-8"));
        assert_eq!(
            ids.iter().filter(|id| **id == "claude-opus-4-8").count(),
            1,
            "curated model must not be duplicated by the live variant"
        );
        let gpt9 = merged.iter().find(|m| m.id == "gpt-9-turbo").unwrap();
        assert_eq!(gpt9.name, "GPT-9 Turbo");
        assert_eq!(gpt9.context_window, Some(500_000));
        assert!(!ids.contains(&"embedding-x"));
    }
}
