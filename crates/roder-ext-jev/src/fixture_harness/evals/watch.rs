//! What a run spent, phase by phase, and what the page said about its own
//! reading, kept beside the result for the eval rows.
//!
//! Measurement only: the browser, the decision client and the text resolver
//! are wrapped and timed from outside, and the observations are read, never
//! changed, so nothing here reaches the decision request. Phases:
//!
//! - `settle`: the wait for the last input's effects, the page's own
//!   `{reason, waited_ms}` kept per look;
//! - `snapshot`: reading the page after it;
//! - `fresh`, `act`, `banner`, `describe`, `screenshot`: the other browser
//!   calls the loop makes, each as a whole (an action that settles inside
//!   itself, or refuses a banner, counts that time to its own phase);
//! - `decide` and `text`: the waits for the decision service and the text
//!   helper.
//!
//! The laps add up to most of the run's own time. What is left over is the
//! loop's code between the calls, plus a settle the page ended on its own.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;

use crate::engine::{JevDecision, JevDecisionClient, JevTextValue, JevTextValueResolver};

/// One phase's calls and wall time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub(crate) struct Lap {
    pub(crate) calls: usize,
    pub(crate) ms: u64,
}

/// How a settle ended: the page's own `{reason, waited_ms}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Settled {
    /// `quiet`, `listbox`, `cap`, or `unanswered` when the page gave none.
    pub(crate) reason: String,
    pub(crate) waited_ms: u64,
}

/// One reading of the page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Looked {
    /// The actions the snapshot offered (keys, scrolls and wait included).
    pub(crate) offered: usize,
    /// The actions it left out: past its caps, hidden, or not reached.
    pub(crate) omitted_actions: u64,
    /// The settle that waited for the previous input's effects before this
    /// reading; absent when no input was pending.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) settle: Option<Settled>,
}

/// A run's laps and readings, as a row carries them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct Watched {
    pub(crate) looks: Vec<Looked>,
    pub(crate) laps: BTreeMap<&'static str, Lap>,
    /// The run's own elapsed time.
    pub(crate) run_ms: u64,
    /// The laps summed.
    pub(crate) attributed_ms: u64,
}

#[derive(Default)]
struct Inner {
    looks: Vec<Looked>,
    laps: BTreeMap<&'static str, (usize, Duration)>,
}

/// The recorder one run shares between its wrappers.
#[derive(Clone, Default)]
pub(crate) struct Watch(Arc<Mutex<Inner>>);

impl Watch {
    pub(crate) fn add(&self, phase: &'static str, took: Duration) {
        let mut inner = self.0.lock().unwrap();
        let lap = inner.laps.entry(phase).or_default();
        lap.0 += 1;
        lap.1 += took;
    }

    /// Time `call` as one call of `phase`.
    pub(crate) async fn time<T>(&self, phase: &'static str, call: impl Future<Output = T>) -> T {
        let started = Instant::now();
        let result = call.await;
        self.add(phase, started.elapsed());
        result
    }

    /// Record a reading, with the settle before it: the page's reply and how
    /// long this side waited, or `None` when no input was pending.
    pub(crate) fn look(&self, observation: &Value, settle: Option<(Value, Duration)>) {
        let settle = settle.map(|(reply, took)| match reply["reason"].as_str() {
            Some(reason) => Settled {
                reason: reason.into(),
                waited_ms: reply["waited_ms"].as_u64().unwrap_or_default(),
            },
            None => Settled {
                reason: "unanswered".into(),
                waited_ms: took.as_millis() as u64,
            },
        });
        self.0.lock().unwrap().looks.push(Looked {
            offered: observation["actions"].as_array().map_or(0, Vec::len),
            omitted_actions: observation["omitted_actions"].as_u64().unwrap_or_default(),
            settle,
        });
    }

    /// Everything recorded so far, for a run that took `run_ms`.
    pub(crate) fn snapshot(&self, run_ms: u64) -> Watched {
        let inner = self.0.lock().unwrap();
        let laps = inner
            .laps
            .iter()
            .map(|(phase, (calls, took))| {
                let ms = took.as_secs_f64() * 1000.0;
                (
                    *phase,
                    Lap {
                        calls: *calls,
                        ms: ms.round() as u64,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        Watched {
            looks: inner.looks.clone(),
            attributed_ms: inner
                .laps
                .values()
                .map(|(_, took)| took.as_secs_f64() * 1000.0)
                .sum::<f64>()
                .round() as u64,
            laps,
            run_ms,
        }
    }

    /// `client`, with every wait for a decision timed as `decide`.
    pub(crate) fn decisions(
        &self,
        client: Arc<dyn JevDecisionClient>,
    ) -> Arc<dyn JevDecisionClient> {
        Arc::new(TimedDecisions {
            inner: client,
            watch: self.clone(),
        })
    }

    /// `resolver`, with every wait for a value timed as `text`.
    pub(crate) fn text(
        &self,
        resolver: Arc<dyn JevTextValueResolver>,
    ) -> Arc<dyn JevTextValueResolver> {
        Arc::new(TimedText {
            inner: resolver,
            watch: self.clone(),
        })
    }
}

struct TimedDecisions {
    inner: Arc<dyn JevDecisionClient>,
    watch: Watch,
}

#[async_trait]
impl JevDecisionClient for TimedDecisions {
    fn uses_images(&self) -> bool {
        self.inner.uses_images()
    }

    async fn choose(
        &self,
        observation: &Value,
        goal: &str,
        history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        self.watch
            .time("decide", self.inner.choose(observation, goal, history))
            .await
    }

    async fn choose_gated(
        &self,
        observation: &Value,
        goal: &str,
        history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        self.watch
            .time(
                "decide",
                self.inner.choose_gated(observation, goal, history),
            )
            .await
    }
}

struct TimedText {
    inner: Arc<dyn JevTextValueResolver>,
    watch: Watch,
}

#[async_trait]
impl JevTextValueResolver for TimedText {
    async fn resolve(&self, field_context: &Value) -> anyhow::Result<JevTextValue> {
        self.watch
            .time("text", self.inner.resolve(field_context))
            .await
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_look_keeps_the_offered_count_the_omitted_count_and_the_settle() {
        let watch = Watch::default();
        watch.look(
            &json!({"actions": [{}, {}, {}], "omitted_actions": 7}),
            Some((
                json!({"reason": "quiet", "waited_ms": 210}),
                Duration::from_millis(215),
            )),
        );
        watch.look(&json!({"actions": []}), None);
        let seen = watch.snapshot(500);
        assert_eq!(
            seen.looks,
            [
                Looked {
                    offered: 3,
                    omitted_actions: 7,
                    settle: Some(Settled {
                        reason: "quiet".into(),
                        waited_ms: 210
                    }),
                },
                Looked {
                    offered: 0,
                    omitted_actions: 0,
                    settle: None,
                },
            ]
        );
    }

    #[test]
    fn a_settle_the_page_did_not_answer_is_not_a_missing_settle() {
        let watch = Watch::default();
        watch.look(
            &json!({"actions": []}),
            Some((Value::Null, Duration::from_millis(130))),
        );
        let seen = watch.snapshot(0);
        assert_eq!(
            seen.looks[0].settle,
            Some(Settled {
                reason: "unanswered".into(),
                waited_ms: 130
            })
        );
    }

    #[test]
    fn laps_count_calls_and_add_up() {
        let watch = Watch::default();
        watch.add("act", Duration::from_micros(400));
        watch.add("act", Duration::from_micros(700));
        watch.add("decide", Duration::from_millis(30));
        let seen = watch.snapshot(40);
        // Sub-millisecond calls are summed before they are rounded.
        assert_eq!(seen.laps["act"], Lap { calls: 2, ms: 1 });
        assert_eq!(seen.laps["decide"], Lap { calls: 1, ms: 30 });
        assert_eq!(seen.attributed_ms, 31);
        assert_eq!(seen.run_ms, 40);
    }

    #[test]
    fn the_row_shape_names_laps_and_leaves_out_a_missing_settle() {
        let watch = Watch::default();
        watch.look(&json!({"actions": [{}], "omitted_actions": 0}), None);
        watch.add("snapshot", Duration::from_millis(12));
        let value = serde_json::to_value(watch.snapshot(20)).unwrap();
        assert_eq!(
            value,
            json!({
                "looks": [{"offered": 1, "omitted_actions": 0}],
                "laps": {"snapshot": {"calls": 1, "ms": 12}},
                "run_ms": 20,
                "attributed_ms": 12,
            })
        );
    }
}
