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

use serde_json::{Value, json};

use super::Page;
use super::act::{ACT_JS, Point};
use crate::engine::StaleObservation;

const UNCOVER_JS: &str = include_str!("../assets/uncover.js");

/// Where a covered target stands after an attempt.
enum Target {
    Covered,
    Free,
    Gone,
}

impl Page {
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

fn point_of(value: &Value) -> Option<Point> {
    Some(Point {
        x: value["x"].as_f64()?,
        y: value["y"].as_f64()?,
    })
}
