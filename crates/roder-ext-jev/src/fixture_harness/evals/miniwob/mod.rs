//! A public-benchmark tier: the hosted model on MiniWoB++ task pages.
//!
//! Opt-in and `#[ignore]`d; it needs a decision key, a text model, Chrome
//! and a MiniWoB++ checkout, which is not vendored:
//!
//! ```text
//! git clone https://github.com/Farama-Foundation/miniwob-plusplus
//! git -C miniwob-plusplus checkout 33c3b4ddef8c6eb67c57a29663d844b1eda7e614
//! JEV_MINIWOB_DIR=$PWD/miniwob-plusplus \
//!   cargo test -p roder-ext-jev miniwob_corpus -- --ignored --nocapture
//! ```
//!
//! The checkout's `miniwob/html/` folder is served from `127.0.0.1:0` and
//! nothing else of it runs here. Every task in
//! `tests/fixtures/evals/miniwob_tasks.json` is an episode per seed; a task
//! labelled unsupported is reported as such and counts as a failure in the
//! overall score, and runs anyway with `JEV_MINIWOB_ATTEMPT_UNSUPPORTED=1`.
//! Values Jev types come from the configured text helper, since they change
//! with the seed. Success is the task's raw reward above zero.
//!
//! - `JEV_MINIWOB_SEEDS` (default `0-4`): a range or a comma list.
//! - `JEV_MINIWOB_TASKS=a,b` narrows the run.
//! - `JEV_MINIWOB_MAX_STEPS` (default 25) caps decisions per episode;
//!   `JEV_MINIWOB_TIMEOUT_S` (default 90) caps each episode's loop.
//! - `JEV_MINIWOB_CONCURRENCY` (default 4): episodes at once, each worker on
//!   its own Chrome.
//!
//! Rows stream to `target/jev-evals/miniwob-<unix seconds>.jsonl`, one per
//! episode, and a summary is printed at the end.

mod episode;
mod report;
mod serve;

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail, ensure};
use serde::Deserialize;

use super::live::{env, live_key};
use super::text_sources::Recorded;
use crate::decide::JevTypeSafeDecisionClient;
use crate::engine::{JevDecisionClient, JevTextValueResolver};
use crate::fixture_harness::{Harness, target_dir};
use crate::text_helper::TextHelper;
use episode::EpisodeSpec;
use report::Row;
use serve::StaticSite;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    pub(super) source: String,
    pub(super) commit: String,
    #[allow(dead_code)]
    license: String,
    #[allow(dead_code)]
    note: String,
    pub(super) excluded: Vec<Excluded>,
    pub(super) tasks: Vec<TaskEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Excluded {
    pub(super) id: String,
    #[allow(dead_code)]
    why: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TaskEntry {
    pub(super) id: String,
    /// Whether Jev can express the task at all.
    pub(super) supported: bool,
    #[allow(dead_code)]
    why: String,
}

fn load_manifest() -> anyhow::Result<Manifest> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/evals/miniwob_tasks.json");
    let raw = std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let manifest: Manifest =
        serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
    let mut ids = manifest
        .tasks
        .iter()
        .map(|task| task.id.as_str())
        .chain(manifest.excluded.iter().map(|task| task.id.as_str()))
        .collect::<Vec<_>>();
    let total = ids.len();
    ids.sort_unstable();
    ids.dedup();
    ensure!(ids.len() == total, "duplicate task ids in the manifest");
    Ok(manifest)
}

/// Narrow to a comma-separated id list; an unknown or excluded id is an error.
fn select<'a>(manifest: &'a Manifest, wanted: Option<&str>) -> anyhow::Result<Vec<&'a TaskEntry>> {
    let all = manifest.tasks.iter().collect::<Vec<_>>();
    let Some(wanted) = wanted.filter(|value| !value.trim().is_empty()) else {
        return Ok(all);
    };
    let wanted = wanted
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .collect::<Vec<_>>();
    for id in &wanted {
        if manifest.excluded.iter().any(|task| task.id == *id) {
            bail!("JEV_MINIWOB_TASKS names {id}, which is excluded");
        }
        ensure!(
            all.iter().any(|task| task.id == *id),
            "JEV_MINIWOB_TASKS names unknown task {id}"
        );
    }
    Ok(all
        .into_iter()
        .filter(|task| wanted.contains(&task.id.as_str()))
        .collect())
}

/// `0-4` or `0,3,7`.
fn parse_seeds(raw: &str) -> anyhow::Result<Vec<u64>> {
    let raw = raw.trim();
    if let Some((low, high)) = raw.split_once('-') {
        let (low, high) = (low.trim().parse::<u64>()?, high.trim().parse::<u64>()?);
        ensure!(low <= high, "empty seed range {raw}");
        return Ok((low..=high).collect());
    }
    let seeds = raw
        .split(',')
        .map(|seed| seed.trim().parse::<u64>())
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("bad JEV_MINIWOB_SEEDS {raw:?}"))?;
    ensure!(!seeds.is_empty(), "no seeds");
    Ok(seeds)
}

fn env_number<T: std::str::FromStr>(key: &str, default: T) -> T {
    env(key)
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(default)
}

struct Setup {
    site: StaticSite,
    decision: Arc<dyn JevDecisionClient>,
    /// `JEV_EVAL_FALLBACK=model`: the fallback after an unfinished episode.
    fallback: Option<Arc<dyn crate::fallback::model::FallbackModel>>,
    text: Arc<TextHelper>,
    max_steps: usize,
    timeout: Duration,
    attempt_unsupported: bool,
    out: Mutex<std::fs::File>,
}

impl Setup {
    fn write(&self, row: &Row) {
        let line = serde_json::to_string(row).expect("serialise a row");
        let mut out = self.out.lock().unwrap();
        writeln!(out, "{line}").expect("write a row");
        out.flush().ok();
    }
}

/// One worker: its own Chrome, episodes off the shared queue until none
/// remain. A setup failure gets one retry on a fresh Chrome, in case the
/// browser, not the task, went wrong.
async fn worker(setup: &Setup, queue: &Mutex<VecDeque<(TaskEntry, u64)>>) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut harness = None;
    loop {
        let Some((task, seed)) = queue.lock().unwrap().pop_front() else {
            return rows;
        };
        if !task.supported && !setup.attempt_unsupported {
            let row = Row::not_attempted(&task, seed, &setup.text.current().label());
            setup.write(&row);
            rows.push(row);
            continue;
        }
        // This episode's text calls, for its row.
        let recorded = Recorded::new(setup.text.clone(), "text-model");
        // The model that wrote this episode's values, read once it ends: an
        // unusable Codex sign-in may have fallen back meanwhile.
        let label = || setup.text.current().label();
        let text: Arc<dyn JevTextValueResolver> = recorded.clone();
        let spec = EpisodeSpec {
            url: setup.site.task_url(&task.id),
            seed,
            max_steps: setup.max_steps,
            timeout: setup.timeout,
            decision: &setup.decision,
            text: &text,
            fallback: setup.fallback.as_ref(),
        };
        let mut attempt = 0;
        let mut row = loop {
            attempt += 1;
            if harness.is_none() {
                harness = Harness::start().await;
            }
            let Some(chrome) = harness.as_ref() else {
                break Row::errored(
                    &task,
                    seed,
                    &label(),
                    &anyhow::anyhow!("Chrome did not start"),
                );
            };
            match episode::run(chrome, &spec).await {
                Ok(episode) => break Row::ran(&task, seed, &label(), episode),
                Err(error) if attempt < 2 => {
                    eprintln!(
                        "{} seed {seed}: {error:#}; retrying on a new Chrome",
                        task.id
                    );
                    harness = None;
                }
                Err(error) => break Row::errored(&task, seed, &label(), &error),
            }
        };
        row.text = recorded.report();
        eprintln!(
            "{:<30} seed {seed} {:<7} {:<14} steps {:>2} {:>6} ms  {}",
            task.id,
            match (row.jev_success, row.success) {
                (true, _) => "ok",
                (false, true) => "ok (fb)",
                (false, false) => "FAIL",
            },
            row.jev_status,
            row.steps,
            row.wall_ms,
            row.cause
        );
        setup.write(&row);
        rows.push(row);
    }
}

/// The production client, or with `JEV_EVAL_VARIANTS=effect` (the only
/// variant MiniWoB honours) one that sends each recent action's effect.
fn decision_client(key: String, model: String) -> Arc<dyn JevDecisionClient> {
    let variants = super::variants::Variants::parse(&env("JEV_EVAL_VARIANTS").unwrap_or_default())
        .expect("JEV_EVAL_VARIANTS");
    if !variants.has(super::variants::Variant::Effect) {
        return Arc::new(JevTypeSafeDecisionClient::new(key, model));
    }
    let http = crate::decide::TypeSafeHttpTransport::new(
        crate::decide::ENDPOINT,
        key,
        crate::http::RetryPolicy::default(),
    );
    super::effect_variant::client(model, Arc::new(http))
}

#[tokio::test]
#[ignore = "public benchmark: needs JEV_MINIWOB_DIR, a decision key, a text model and Chrome"]
async fn miniwob_corpus() {
    let manifest = load_manifest().unwrap();
    let tasks = select(&manifest, env("JEV_MINIWOB_TASKS").as_deref()).unwrap();
    let Some(dir) = env("JEV_MINIWOB_DIR").map(PathBuf::from) else {
        eprintln!("skipping: set JEV_MINIWOB_DIR to a miniwob-plusplus checkout");
        return;
    };
    let root = dir.join("miniwob/html");
    assert!(
        root.join("core/core.js").is_file(),
        "{} is not a miniwob-plusplus checkout",
        dir.display()
    );
    let Some(key) = live_key() else {
        eprintln!("skipping: set TYPESAFE_API_KEY (or JEV_API_KEY)");
        return;
    };
    let text_model = crate::runner::resolve_text_model(None)
        .await
        .unwrap()
        .expect("MiniWoB values change with the seed: configure Jev's text model");
    let text_label = text_model.label();
    let seeds = parse_seeds(&env("JEV_MINIWOB_SEEDS").unwrap_or_else(|| "0-4".into())).unwrap();
    let model = env("JEV_MODEL").unwrap_or_else(|| "jev-latest".into());
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let dir = target_dir().join("jev-evals");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("miniwob-{stamp}.jsonl"));
    let setup = Setup {
        site: StaticSite::start(root).await.unwrap(),
        decision: decision_client(key, model.clone()),
        fallback: super::fallback_live::live_fallback().await,
        text: Arc::new(TextHelper::new(text_model)),
        max_steps: env_number("JEV_MINIWOB_MAX_STEPS", 25usize).max(1),
        timeout: Duration::from_secs(env_number("JEV_MINIWOB_TIMEOUT_S", 90u64).max(1)),
        attempt_unsupported: env("JEV_MINIWOB_ATTEMPT_UNSUPPORTED").as_deref() == Some("1"),
        out: Mutex::new(std::fs::File::create(&path).unwrap()),
    };
    let concurrency = env_number("JEV_MINIWOB_CONCURRENCY", 4usize).max(1);
    eprintln!(
        "decision model {model}, text model {}, fallback {}, seeds {seeds:?}, {} decisions and \
         {:?} per episode, {concurrency} workers, unsupported attempted: {}\nrows: {}",
        text_label,
        setup
            .fallback
            .as_ref()
            .map_or("off".to_string(), |model| model.label()),
        setup.max_steps,
        setup.timeout,
        setup.attempt_unsupported,
        path.display()
    );
    let queue = Mutex::new(
        tasks
            .iter()
            .flat_map(|task| seeds.iter().map(|seed| ((*task).clone(), *seed)))
            .collect::<VecDeque<_>>(),
    );
    let started = Instant::now();
    let rows = futures::future::join_all((0..concurrency).map(|_| worker(&setup, &queue)))
        .await
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    eprintln!(
        "\n{}total wall time {:.1} s\nrows: {}",
        report::summary(&manifest, &tasks, &rows),
        started.elapsed().as_secs_f64(),
        path.display()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manifest_names_every_task_once() {
        let manifest = load_manifest().unwrap();
        assert_eq!(manifest.tasks.len() + manifest.excluded.len(), 130);
        assert_eq!(
            manifest
                .excluded
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            ["text-transform"]
        );
        assert!(manifest.tasks.iter().any(|task| task.supported));
        assert!(manifest.tasks.iter().any(|task| !task.supported));
    }

    #[test]
    fn a_task_filter_must_name_runnable_tasks() {
        let manifest = load_manifest().unwrap();
        assert_eq!(select(&manifest, None).unwrap().len(), manifest.tasks.len());
        let ids = select(&manifest, Some("click-test, enter-text"))
            .unwrap()
            .into_iter()
            .map(|task| task.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["click-test", "enter-text"]);
        assert!(select(&manifest, Some("click_test")).is_err());
        assert!(select(&manifest, Some("text-transform")).is_err());
    }

    #[test]
    fn seeds_parse_as_a_range_or_a_list() {
        assert_eq!(parse_seeds("0-4").unwrap(), [0, 1, 2, 3, 4]);
        assert_eq!(parse_seeds("7, 3").unwrap(), [7, 3]);
        assert!(parse_seeds("4-0").is_err());
        assert!(parse_seeds("a").is_err());
    }
}
