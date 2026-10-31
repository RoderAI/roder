//! The second opinion the loop may ask for (after fastbrowse's `consult` and
//! light `recover`): one text-model call that names the next control, when
//! Jev is unsure of a step that acts, is about to repeat the click it just
//! made, or has stopped making progress. Whatever it names still passes
//! every gate Jev's own choice would, and it never ends a run itself.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::JevEngineConfig;

/// Below this call confidence (the least certain of Jev's operation and
/// target heads), a step that acts is checked with the consultant first.
/// Zero by default, so only an unsure repeat is asked about: at
/// fastbrowse's 0.55 the consult swapped Jev's choice on 29 of 231 live
/// corpus runs, and its swaps cost three passes and added two false greens
/// (MiniWoB++ moved 86 to 87 of 96, within noise). `JEV_CONSULT_BELOW=0.55`
/// turns it on.
pub const CONSULT_BELOW: f64 = 0.0;

/// Stalls a run may get a one-step recovery for before it ends `blocked`.
/// Zero by default: at 2 the tier fired on 9 of 231 live runs and 1 of 96
/// MiniWoB++ episodes and turned no failure into a pass.
/// `JEV_LIGHT_RECOVERIES=2` turns it on.
pub const LIGHT_RECOVERIES: usize = 0;

/// The consultant's answer: one observed control's id, or none.
#[derive(Debug, Clone)]
pub struct JevConsultAnswer {
    /// The id of one control the request listed, or `None` when no listed
    /// control helps (the task needs the user, or is already done).
    pub control: Option<String>,
    /// Why, in a line; page text, scrubbed before it is reported.
    pub diagnosis: String,
    pub model: String,
    pub latency_ms: u64,
    pub usage: Value,
}

/// Names the next control from the request the loop builds: the goal, the
/// page's url, title and text, its controls, the recent steps, Jev's
/// proposal, and the problem. Roder's text helper is one. An error is no
/// advice: the loop goes on as if it had not asked.
#[async_trait]
pub trait JevConsultant: Send + Sync {
    async fn consult(&self, request: &Value) -> anyhow::Result<JevConsultAnswer>;
}

/// The consultant and when the loop asks it.
#[derive(Clone)]
pub(crate) struct Consult {
    pub(crate) consultant: Option<Arc<dyn JevConsultant>>,
    pub(crate) below: f64,
    pub(crate) light_recoveries: usize,
}

impl Default for Consult {
    fn default() -> Self {
        Self {
            consultant: None,
            below: CONSULT_BELOW,
            light_recoveries: LIGHT_RECOVERIES,
        }
    }
}

impl JevEngineConfig {
    /// Ask `consultant` before a step Jev is unsure of
    /// ([`with_consult_below`](Self::with_consult_below)) or before an
    /// unsure repeat of the click just made, and give a stalled run up to
    /// [`with_light_recoveries`](Self::with_light_recoveries) one-step
    /// recoveries before it ends `blocked`. Off without a consultant.
    pub fn with_consultant(mut self, consultant: Arc<dyn JevConsultant>) -> Self {
        self.consult.consultant = Some(consultant);
        self
    }

    /// The call confidence under which a step that acts is checked first;
    /// [`CONSULT_BELOW`] (`0`, only repeats are asked about) by default.
    pub fn with_consult_below(mut self, below: f64) -> Self {
        self.consult.below = below.clamp(0.0, 1.0);
        self
    }

    /// How many stalls get a one-step recovery; [`LIGHT_RECOVERIES`] (`0`,
    /// every stall ends as before) by default.
    pub fn with_light_recoveries(mut self, recoveries: usize) -> Self {
        self.consult.light_recoveries = recoveries;
        self
    }
}
