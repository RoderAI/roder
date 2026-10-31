//! Dismissing what covers a target before giving up on it.
//!
//! A covered target used to be a step that changed nothing, and three of
//! them ended the run `blocked`. The live booking benchmark ended that way:
//! Jev had opened a date picker itself, the picker stayed open over the
//! results, and every click on a restaurant's slot was covered. Now, when
//! act.js finds the target covered by a layer laid over the page (a
//! popover, menu, picker or dialog; see `uncover.js`), Jev tries, in order
//! and checking the target after each: Escape; the layer's own close
//! control, a button whose whole name is a close word; a press outside the
//! layer on nothing that acts (or on a full-page backdrop). The first that
//! uncovers the target is recorded on the step and the action goes ahead in
//! the same step, with no model call. None of them presses anything that
//! accepts, confirms or commits. A cookie or consent banner is left to
//! banner refusal, and a layer that stayed is not tried again in the run.
//!
//! A band fixed to the window (an anchored ad, a sticky footer, a chat
//! bubble; see `band.js`) is not dismissed at all: it covers a place on the
//! screen, not a control, so the target is scrolled to the centre, the start
//! and the end of what scrolls it in turn, and pressed where a point of it
//! is its own again. Where the page cannot scroll it clear (a page that does
//! not scroll), it is pressed from the keyboard, as a keyboard user would,
//! with the key its semantics answer to: the target is focused, and right
//! before the key the control's guard and what covers it are checked again,
//! since focus and scroll handlers can repurpose the control or open a menu
//! or dialog over it. Nothing in the band is ever pressed, and a cookie or
//! consent bar is no band: it is left to banner refusal. The rules follow
//! fastbrowse's page/act.rs (`route_around`, `keys_reach`; MIT).

use serde_json::{Value, json};

use super::Page;
use super::act::{
    ACT_JS, FOCUS_JS, FRAME_JS, Point, SELECT_JS, control_state, select_outcome, target_point,
};
use crate::engine::{Covered, JevActOutcome, StaleObservation};

const UNCOVER_JS: &str = include_str!("../assets/uncover.js");
const BAND_JS: &str = include_str!("../assets/band.js");

/// How a covered target can still be reached.
pub(super) enum Reach {
    /// Scrolled clear of a band: the hit test that found a point of it.
    Clear(Value),
    /// A band still covers it: press it from the keyboard.
    Keyboard,
    /// Not a band: dismiss what covers it (`uncover`), or report it.
    Cover,
}

/// Where a covered target stands after an attempt.
enum Target {
    Covered,
    Free,
    Gone,
}

impl Page {
    /// Where a target `hit_test` found covered can be reached: scrolled
    /// clear of a band fixed to the window, from the keyboard under one, or
    /// not past a band at all.
    pub(super) async fn route_around(
        &mut self,
        action: &Value,
        hit_test: &str,
    ) -> anyhow::Result<Reach> {
        if self.cover_kind(action).await?.0 != "band" {
            return Ok(Reach::Cover);
        }
        for block in ["center", "start", "end"] {
            let there = self
                .evaluate(&format!(
                    "({BAND_JS}).align({})",
                    json!({"node": action["node"], "block": block})
                ))
                .await?;
            if there != json!(true) {
                return Err(gone());
            }
            // The page's scroll handlers run before the target is measured.
            self.evaluate_async(&format!("({FRAME_JS})()")).await?;
            let hit = self.evaluate_async(hit_test).await?;
            if hit.is_null() {
                return Err(gone());
            }
            if hit["covered"] != json!(true) {
                return Ok(Reach::Clear(hit));
            }
        }
        // The last alignment can have put the target under something else.
        Ok(match self.cover_kind(action).await?.0.as_str() {
            "band" => Reach::Keyboard,
            _ => Reach::Cover,
        })
    }

    /// Click, fill or choose `action`'s target from the keyboard while a band
    /// covers it. `guard` is the control's state as the decision saw it, and
    /// `by` the name act.js gave the band. A control whose semantics answer
    /// to no key, that will not take focus, that changed, or that something
    /// other than a band covers once focused, is not pressed.
    pub(super) async fn through_keys(
        &mut self,
        action: &Value,
        guard: &Value,
        by: Option<&str>,
        text: Option<&str>,
        settle: &mut Value,
    ) -> anyhow::Result<JevActOutcome> {
        let band = by.map_or("a band fixed to the window".to_string(), |by| {
            format!("\"{}\"", by.chars().take(60).collect::<String>())
        });
        let kind = action["kind"].as_str().unwrap_or_default();
        let key = match kind {
            "click" => {
                let key = self
                    .evaluate(&format!(
                        "({BAND_JS}).key({})",
                        json!({"node": action["node"]})
                    ))
                    .await?;
                match key.as_str() {
                    Some(key @ ("Enter" | " ")) => key.to_string(),
                    _ => return Err(self.covered_now(action).await),
                }
            }
            "fill" | "select" => String::new(),
            _ => return Err(self.covered_now(action).await),
        };
        let focused = self.evaluate(&format!("({FOCUS_JS})({action})")).await?;
        if focused.is_null() {
            return Err(gone());
        }
        if focused != json!(true) {
            return Err(self.covered_now(action).await);
        }
        let at = self.keys_reach(action, guard).await?;
        let (mut outcome, how) = match kind {
            "select" => {
                let chosen = self.evaluate(&format!("({SELECT_JS})({action})")).await?;
                (select_outcome(action, &chosen)?, "chose from it by script")
            }
            "fill" => (
                self.fill(action, text.unwrap_or_default(), at, settle)
                    .await?,
                "typed into it from the keyboard",
            ),
            _ => {
                self.press_key(&key).await?;
                (JevActOutcome::done(), "pressed it from the keyboard")
            }
        };
        outcome.uncovered = Some(format!("{how}: {band} covers it"));
        Ok(outcome)
    }

    /// Whether a key sent now goes to the control the decision chose: the
    /// control unchanged, and nothing but a band (or nothing) over it once
    /// focused. Focus and scroll handlers can repurpose the control, or open
    /// a menu or dialog over it that keys would get around. The target's
    /// centre, for a fill's hand-off, when they do.
    pub(super) async fn keys_reach(
        &mut self,
        action: &Value,
        guard: &Value,
    ) -> anyhow::Result<Point> {
        self.evaluate_async(&format!("({FRAME_JS})()")).await?;
        let live = self
            .evaluate(&format!(
                "(a => {{ const c=window.__jevFast; \
                 return c?.guard ? c.guard(c.nodes.get(a.node)) : null; }})({action})"
            ))
            .await?;
        if !guard.is_null() && &control_state(&live) != guard {
            return Err(StaleObservation::new(
                "The control changed before the key press. Observe again.",
            )
            .into());
        }
        match self.cover_kind(action).await? {
            (kind, Some(at)) if kind == "band" || kind == "none" => Ok(at),
            (kind, _) if kind == "gone" => Err(gone()),
            _ => Err(self.covered_now(action).await),
        }
    }

    /// What covers the target's centre (`band.js`): `none`, `band`,
    /// `obstruction`, `consent`, `other`, or `gone`, with the centre.
    async fn cover_kind(&mut self, action: &Value) -> anyhow::Result<(String, Option<Point>)> {
        let kind = self
            .evaluate(&format!(
                "({BAND_JS}).kind({})",
                json!({"node": action["node"]})
            ))
            .await?;
        Ok((
            kind["kind"].as_str().unwrap_or("gone").to_string(),
            point_of(&kind),
        ))
    }

    /// The refusal for a target that stays covered, naming what covers it
    /// now as act.js reads it.
    async fn covered_now(&mut self, action: &Value) -> anyhow::Error {
        let covered = || Covered::new("Another element covers the target; nothing was clicked.");
        match self.evaluate(&format!("({ACT_JS})({action})")).await {
            Ok(hit) if hit["covered"] == json!(true) => {
                target_point(&hit).err().unwrap_or_else(|| covered().into())
            }
            _ => covered().into(),
        }
    }

    /// Try to dismiss what covers `action`'s target. How it went, when the
    /// target is free now; `None` when nothing worked or nothing was tried.
    pub(super) async fn uncover(&mut self, action: &Value) -> anyhow::Result<Option<String>> {
        let Some(plan) = self.plan_uncover(action).await? else {
            return Ok(None);
        };
        let cover = plan["cover"].as_str().unwrap_or("a popup").to_string();
        self.press_key("Escape").await?;
        if self.freed(action).await? {
            return Ok(Some(format!("pressed Escape to close \"{cover}\"")));
        }
        // The layer may have changed under the key; ask again.
        let plan = self.plan_uncover(action).await?.unwrap_or(plan);
        if let Some(close) = point_of(&plan["close"]) {
            self.press_at(close).await?;
            if self.freed(action).await? {
                let label = plan["close"]["label"].as_str().unwrap_or("close");
                return Ok(Some(format!("clicked \"{label}\" to close \"{cover}\"")));
            }
        }
        let plan = self.plan_uncover(action).await?.unwrap_or(plan);
        if let Some(outside) = point_of(&plan["outside"]) {
            self.press_at(outside).await?;
            if self.freed(action).await? {
                return Ok(Some(format!("clicked outside \"{cover}\" to close it")));
            }
        }
        self.evaluate(&format!(
            "({UNCOVER_JS})({})",
            json!({"node": action["node"], "phase": "failed"})
        ))
        .await?;
        Ok(None)
    }

    /// What covers the target and how it might go, or `None` when it is
    /// not a layer Jev may dismiss.
    async fn plan_uncover(&mut self, action: &Value) -> anyhow::Result<Option<Value>> {
        let plan = self
            .evaluate(&format!(
                "({UNCOVER_JS})({})",
                json!({"node": action["node"], "phase": "plan"})
            ))
            .await?;
        let usable =
            plan.is_object() && plan["consent"] != json!(true) && plan["stuck"] != json!(true);
        Ok(usable.then_some(plan))
    }

    /// Let the attempt land, then hit-test the target again. A target that
    /// went away with the layer is a changed page, not a success.
    async fn freed(&mut self, action: &Value) -> anyhow::Result<bool> {
        self.settle_page(None).await;
        match self.target_now(action).await? {
            Target::Free => Ok(true),
            Target::Covered => Ok(false),
            Target::Gone => Err(StaleObservation::new(
                "The target went away while Jev closed what covered it. Observe again.",
            )
            .into()),
        }
    }

    async fn target_now(&mut self, action: &Value) -> anyhow::Result<Target> {
        let hit = self.evaluate(&format!("({ACT_JS})({action})")).await?;
        Ok(match hit {
            Value::Null => Target::Gone,
            hit if hit["covered"] == json!(true) => Target::Covered,
            _ => Target::Free,
        })
    }
}

fn gone() -> anyhow::Error {
    StaleObservation::new(
        "The target went away while Jev reached past what covers it. Observe again.",
    )
    .into()
}

fn point_of(value: &Value) -> Option<Point> {
    Some(Point {
        x: value["x"].as_f64()?,
        y: value["y"].as_f64()?,
    })
}
