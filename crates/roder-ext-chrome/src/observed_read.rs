//! Which parts of a page an observation actually read.
//!
//! A snapshot asked for only some sections (`include`) comes back with the
//! others empty, and empty is not the same as nothing there: saying "no
//! controls" would be false, and comparing the next page with it would find
//! every control new. [`Read`] records which sections were read, so a render
//! can say a section was not requested, [`compare`](crate::observed::compare)
//! leaves it out, and a partial page does not replace the fuller one it is
//! compared with next.

use serde_json::Value;

use crate::observed::Observed;

/// The sections of a page an observation carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Read {
    pub(crate) controls: bool,
    pub(crate) text: bool,
}

impl Default for Read {
    /// A look, an action's observation and a whole snapshot read it all.
    fn default() -> Self {
        Self {
            controls: true,
            text: true,
        }
    }
}

impl Observed {
    /// This page as a snapshot asked for only the sections in `include`
    /// (the extension's `SnapshotInclude` names) read it.
    pub(crate) fn limited_to(mut self, include: &[Value]) -> Self {
        let asked = |name: &str| include.iter().any(|section| section == name);
        self.read = Read {
            controls: asked("controls"),
            text: asked("text"),
        };
        self
    }

    /// This page with the sections it did not read taken from `earlier`, when
    /// that is the same page (same address) and did read them: what is kept to
    /// compare the next page with is the fullest page known, not the latest
    /// partial one.
    pub(crate) fn filled_from(mut self, earlier: Option<&Observed>) -> Self {
        let Some(earlier) = earlier.filter(|earlier| earlier.url == self.url) else {
            return self;
        };
        if !self.read.controls && earlier.read.controls {
            self.controls = earlier.controls.clone();
            self.read.controls = true;
        }
        if !self.read.text && earlier.read.text {
            self.text = earlier.text.clone();
            self.text_cut = earlier.text_cut;
            self.read.text = true;
        }
        self
    }
}
