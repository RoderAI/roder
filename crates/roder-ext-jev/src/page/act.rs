//! Executing one observed action.
//!
//! Jev's own, diverging from upstream, which pressed the mouse at the point
//! it hit-tested straight away. A click or a fill first moves the pointer
//! there, lets a frame render, and checks in one evaluate that the target is
//! unchanged and still under that point; a target that moved is followed for
//! up to a second before anything is pressed, so a layout shift cannot send
//! the press to whatever took its place, and hover handlers run first as they
//! would for a person. The re-check compares the control's own state, not
//! the text around it (the guard's last entry): the freshness check read
//! that text just before, and a tooltip the pointer reveals in a row, or a
//! ticker beside a field, does not make the target another one. A select
//! reports a choice its page put back. The pointer rule follows fastbrowse's
//! browser/page.py (MIT), which compares the whole guard.

use std::time::Duration;

use anyhow::bail;
use serde_json::{Value, json};

use super::Page;
use super::uncover::Reach;
use crate::engine::{Covered, JevActOutcome, StaleObservation};

pub(super) const ACT_JS: &str = include_str!("../assets/act.js");
pub(super) const SELECT_JS: &str = include_str!("../assets/select.js");
pub(super) const FRAME_JS: &str = include_str!("../assets/frame.js");
/// Focuses an observed text field for Enter without selecting its text;
/// whether it holds focus, or null when it is gone or hidden.
pub(super) const FOCUS_JS: &str = "(a => { const c=window.__jevFast, e=c?.nodes.get(a.node); \
    if (!c?.tree || !e?.isConnected || !e.checkVisibility()) return null; \
    if (c.tree.active()!==e) e.focus({preventScroll:true}); return c.tree.active()===e; })";
/// Scrolls an observed box inside itself by four fifths of its height, and
/// at least 320 px, so a small box full of text is not a dozen steps; whether
/// it moved, or null when it is gone or hidden.
const REGION_JS: &str = "(a => { const e=window.__jevFast?.nodes.get(a.node); \
    if (!e?.isConnected || !e.checkVisibility()) return null; const before=e.scrollTop; \
    e.scrollBy({top:(a.direction==='up' ? -1 : 1)*Math.round(Math.max(e.clientHeight*0.8,320)), \
    behavior:'instant'}); return e.scrollTop!==before; })";

/// How long a target may keep moving under the pointer before the press is
/// given up.
const STABLE_WITHIN: Duration = Duration::from_secs(1);
/// Points closer than this are the same point.
const SAME_POINT_PX: f64 = 0.5;

/// Where a press landed, for the fill that follows it.
#[derive(Debug, Clone, Copy)]
pub(super) struct Point {
    pub(super) x: f64,
    pub(super) y: f64,
}

impl Point {
    fn of(hit: &Value) -> Option<Self> {
        Some(Self {
            x: hit["x"].as_f64()?,
            y: hit["y"].as_f64()?,
        })
    }

    fn near(self, other: Self) -> bool {
        (self.x - other.x).abs() < SAME_POINT_PX && (self.y - other.y).abs() < SAME_POINT_PX
    }
}

impl Page {
    /// Execute one observed action.
    pub(crate) async fn act(
        &mut self,
        action: &Value,
        observation: &Value,
        text: Option<&str>,
        wait: Duration,
    ) -> anyhow::Result<JevActOutcome> {
        let changes = self.freshness_changes(observation, Some(action)).await?;
        if !changes.is_empty() {
            return Err(StaleObservation::new(format!(
                "Page changed since this decision: {}. Observe again.",
                changes.join(", ")
            ))
            .into());
        }
        // Inputs before this action (a refused banner, an uncovered layer
        // between reads) are not its own.
        self.connection.take_navigation_mark();
        let kind = action["kind"].as_str().unwrap_or_default();
        let mut settle = action.clone();
        let outcome = match kind {
            "wait" => {
                tokio::time::sleep(wait).await;
                JevActOutcome::done()
            }
            "scroll" if action["node"].is_i64() => self.scroll_region(action).await?,
            "enter" => self.press_enter(action, observation).await?,
            "key" => {
                self.press_key("Escape").await?;
                JevActOutcome::done()
            }
            "scroll" => {
                // The window itself, by script: a wheel at a fixed point
                // scrolled whatever sat there instead (a map, a list in a
                // panel).
                let delta = action["delta"].as_f64().unwrap_or_default();
                self.evaluate(&format!(
                    "window.scrollBy({{top:{delta},behavior:'instant'}})"
                ))
                .await?;
                JevActOutcome::done()
            }
            _ => {
                self.dispatch_input(action, observation, text, &mut settle)
                    .await?
            }
        };
        self.after_input = (kind != "wait").then_some(settle);
        Ok(outcome)
    }

    /// Scroll a box that scrolls inside itself. A box that did not move (already at its end, or a page that pins it)
    /// is a refused step.
    async fn scroll_region(&mut self, action: &Value) -> anyhow::Result<JevActOutcome> {
        let moved = self.evaluate(&format!("({REGION_JS})({action})")).await?;
        match moved.as_bool() {
            Some(true) => Ok(JevActOutcome::done()),
            Some(false) => Ok(JevActOutcome::refused("The box did not scroll.")),
            None => {
                Err(StaleObservation::new("The box to scroll went away. Observe again.").into())
            }
        }
    }

    /// Press Enter in the observed text field, focused first if focus left
    /// it. A field that is gone, or will not take focus, is not pressed, and
    /// neither is one that changed or that something other than a band
    /// fixed to the window covers once focused: Enter would get around it
    /// (see `keys_reach`).
    async fn press_enter(
        &mut self,
        action: &Value,
        observation: &Value,
    ) -> anyhow::Result<JevActOutcome> {
        let focused = self.evaluate(&format!("({FOCUS_JS})({action})")).await?;
        match focused.as_bool() {
            Some(true) => {
                let guard = observation["guards"]
                    .get(action["node"].to_string())
                    .map(control_state)
                    .unwrap_or(Value::Null);
                self.keys_reach(action, &guard).await?;
                self.press_key("Enter").await?;
                Ok(JevActOutcome::done())
            }
            Some(false) => Ok(JevActOutcome::refused(
                "The field did not take keyboard focus; Enter was not pressed.",
            )),
            None => Err(StaleObservation::new("The field went away. Observe again.").into()),
        }
    }

    /// A key press and release to whatever has focus, as a keyboard sends it.
    ///
    /// No native key code is sent: native codes are the platform's, and
    /// Escape's Windows code 27 sent as macOS's native code opened Chrome's
    /// "About Chrome" page from a shown tab. Escape, which types nothing,
    /// goes as a `rawKeyDown`. Space is `" "`, as the DOM names it.
    pub(super) async fn press_key(&mut self, key: &str) -> anyhow::Result<()> {
        let (kind, text, code, name) = match key {
            "Enter" => ("keyDown", "\r", 13, "Enter"),
            " " => ("keyDown", " ", 32, "Space"),
            _ => ("rawKeyDown", "", 27, "Escape"),
        };
        let mut down = json!({"type": kind, "key": key, "code": name,
            "windowsVirtualKeyCode": code});
        if !text.is_empty() {
            down["text"] = json!(text);
            down["unmodifiedText"] = json!(text);
        }
        self.call("Input.dispatchKeyEvent", down).await?;
        self.call(
            "Input.dispatchKeyEvent",
            json!({"type": "keyUp", "key": key, "code": name, "windowsVirtualKeyCode": code}),
        )
        .await
        .map(|_| ())
    }

    /// Hit-test the target, then select, click or fill it. `settle` is the
    /// action the next settle waits on; a click records whether it announced
    /// a popup, a fill the field that took the text.
    async fn dispatch_input(
        &mut self,
        action: &Value,
        observation: &Value,
        text: Option<&str>,
        settle: &mut Value,
    ) -> anyhow::Result<JevActOutcome> {
        if !action["node"].is_i64() {
            bail!("Invalid observed node");
        }
        let kind = action["kind"].as_str().unwrap_or_default();
        let hit_test = format!(
            "(async a => {{ const p=({ACT_JS})(a); if (p && !p.covered) {{ \
             p.hidden=document.hidden; \
             if (a.kind==='select') p.chosen=({SELECT_JS})(a); \
             else if (p.scrolled) await ({FRAME_JS})(); }} return p; }})({action})"
        );
        let guard = observation["guards"]
            .get(action["node"].to_string())
            .map(control_state)
            .unwrap_or(Value::Null);
        let mut hit = self.evaluate_async(&hit_test).await?;
        // A target under a band fixed to the window is scrolled clear of it,
        // or pressed from the keyboard; one under a popover, menu or dialog
        // has that dismissed first. Either goes ahead in the same step when
        // the target is free (see `uncover`).
        let mut uncovered = None;
        if hit["covered"] == json!(true) {
            match self.route_around(action, &hit_test).await? {
                Reach::Clear(clear) => {
                    uncovered = Some(match hit["by"].as_str() {
                        Some(by) => format!(
                            "scrolled it clear of \"{}\"",
                            by.chars().take(60).collect::<String>()
                        ),
                        None => "scrolled it clear of a band fixed to the window".to_string(),
                    });
                    hit = clear;
                }
                Reach::Keyboard => {
                    // Enter on a link naming `_blank` opens its tab as a
                    // click does: the settle waits for it the same way.
                    if hit["opens"] == json!(true) {
                        settle["opens"] = json!(true);
                    }
                    return self
                        .through_keys(action, &guard, hit["by"].as_str(), text, settle)
                        .await;
                }
                Reach::Cover => {
                    if let Some(how) = self.uncover(action).await? {
                        uncovered = Some(how);
                        hit = self.evaluate_async(&hit_test).await?;
                    }
                }
            }
        }
        let point = target_point(&hit)?;
        let outcome = |mut outcome: JevActOutcome| {
            outcome.uncovered = uncovered.clone();
            outcome
        };
        if kind == "select" {
            return select_outcome(action, &hit["chosen"]).map(outcome);
        }
        if hit["announced"] == json!(true) {
            settle["announced"] = json!(true);
        }
        if hit["opens"] == json!(true) {
            settle["opens"] = json!(true);
        }
        let point = match hit["hidden"] == json!(true) {
            true => self.press_at(point).await?,
            false => self.press(action, &guard, point).await?,
        };
        if kind != "fill" {
            return Ok(outcome(JevActOutcome::done()));
        }
        // The press has been sent, so from here the action ran: a page that
        // changed under it (the click navigated) is a step that typed
        // nothing, recorded like any refusal, not a stale decision the loop
        // would drop from its history, budgets and stall rule.
        match self
            .fill(action, text.unwrap_or_default(), point, settle)
            .await
        {
            Err(error) if error.is::<StaleObservation>() => Ok(outcome(JevActOutcome::refused(
                format!("The page changed after the click ({error}); nothing was typed."),
            ))),
            result => result.map(outcome),
        }
    }

    /// Move the pointer to the target, wait for a frame, and press only once
    /// the unchanged target stays under the pointer.
    async fn press(
        &mut self,
        action: &Value,
        guard: &Value,
        mut point: Point,
    ) -> anyhow::Result<Point> {
        let deadline = tokio::time::Instant::now() + STABLE_WITHIN;
        loop {
            self.mouse("mouseMoved", point).await?;
            let checked = self
                .evaluate_async(&format!(
                    "(async a => {{ await ({FRAME_JS})(); const c=window.__jevFast; \
                     return [c?.guard ? c.guard(c.nodes.get(a.node)) : null, ({ACT_JS})(a)]; }})({action})"
                ))
                .await?;
            if &control_state(&checked[0]) != guard {
                return Err(StaleObservation::new(
                    "The control changed before the press. Observe again.",
                )
                .into());
            }
            let now = target_point(&checked[1])?;
            if now.near(point) {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(StaleObservation::new(
                    "The target did not stop moving; nothing was clicked.",
                )
                .into());
            }
            point = now;
        }
        self.press_at(point).await
    }

    /// Press and release at `point`. A hidden tab (focus emulation failed)
    /// gets only this: it draws no frames, so nothing on it moves under the
    /// pointer, and Chrome holds a pointer move's reply for 5 s waiting for
    /// one (`fixture_harness::hidden_tab_tests`).
    pub(super) async fn press_at(&mut self, point: Point) -> anyhow::Result<Point> {
        for event in ["mousePressed", "mouseReleased"] {
            self.mouse(event, point).await?;
        }
        Ok(point)
    }

    async fn mouse(&mut self, event: &str, point: Point) -> anyhow::Result<()> {
        let mut params = json!({"type": event, "x": point.x, "y": point.y});
        if event != "mouseMoved" {
            params["button"] = json!("left");
            params["clickCount"] = json!(1);
        }
        self.call("Input.dispatchMouseEvent", params)
            .await
            .map(|_| ())
    }
}

/// A guard without its last entry, the text of the row or form around the
/// control: the control's identity, name, value and ARIA state.
pub(super) fn control_state(guard: &Value) -> Value {
    match guard.as_array() {
        Some(guard) => Value::Array(guard[..guard.len().saturating_sub(1)].to_vec()),
        None => Value::Null,
    }
}

/// The point act.js found, or why there is none: covered, or a stale page.
/// A target that is covered carries the name act.js read off what covers it.
pub(super) fn target_point(hit: &Value) -> anyhow::Result<Point> {
    if hit["covered"] == json!(true) {
        let controls = hit["controls"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string);
        let covered = Covered::new("Another element covers the target; nothing was clicked.")
            .with_controls(controls);
        return Err(match hit["by"].as_str() {
            Some(cover) => covered.with_cover(cover),
            None => covered,
        }
        .into());
    }
    Point::of(hit).ok_or_else(|| StaleObservation::new("Target changed. Observe again.").into())
}

/// What select.js reported: a choice the page kept, one it put back, or
/// none made at all.
pub(super) fn select_outcome(action: &Value, chosen: &Value) -> anyhow::Result<JevActOutcome> {
    if chosen.is_null() {
        bail!("Dropdown execution was not confirmed; inspect before retrying.");
    }
    if chosen["kept"] == json!(true) {
        return Ok(JevActOutcome::done());
    }
    let label = action["label"].as_str().unwrap_or_default();
    let option = label.split(" → ").nth(1).unwrap_or(label);
    let shown = chosen["shown"].as_str().unwrap_or_default();
    Ok(JevActOutcome::refused(format!(
        "The page kept {shown:?} instead of {option:?}."
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hit_is_a_point_covered_or_stale() {
        let point = target_point(&json!({"x": 10.5, "y": 20, "announced": false})).unwrap();
        assert_eq!((point.x, point.y), (10.5, 20.0));
        let covered = target_point(&json!({"covered": true})).unwrap_err();
        assert!(covered.is::<Covered>(), "{covered:#}");
        // Nothing hit, or nothing named: a covered target without a name.
        assert_eq!(covered.downcast_ref::<Covered>().unwrap().cover(), None);
        let stale = target_point(&Value::Null).unwrap_err();
        assert!(stale.is::<StaleObservation>(), "{stale:#}");
    }

    #[test]
    fn a_covered_hit_carries_the_name_of_what_covers_the_target() {
        let named = target_point(&json!({"covered": true, "by": "Spring sale popup"}));
        let error = named.unwrap_err();
        let covered = error.downcast_ref::<Covered>().expect("covered");
        assert_eq!(covered.cover(), Some("Spring sale popup"));
        // The message stays fixed text: the name is page text and has its
        // own accessor, scrubbed by the loop before it is recorded.
        assert_eq!(
            covered.to_string(),
            "Another element covers the target; nothing was clicked."
        );
        for by in [json!(null), json!(""), json!("  "), json!(7)] {
            let error = target_point(&json!({"covered": true, "by": by})).unwrap_err();
            assert_eq!(
                error.downcast_ref::<Covered>().unwrap().cover(),
                None,
                "{by}"
            );
        }
        // A hit that is a point has no cover, whatever else it carries.
        assert!(target_point(&json!({"x": 1, "y": 2, "by": "x"})).is_ok());
    }

    #[test]
    fn a_covered_hit_carries_what_its_cover_offers_to_press() {
        let hit = json!({"covered": true, "by": "Offer",
            "controls": ["Join", "", 7, "No thanks, take me to the shop"]});
        let error = target_point(&hit).unwrap_err();
        let covered = error.downcast_ref::<Covered>().expect("covered");
        assert_eq!(
            covered.controls(),
            ["Join", "No thanks, take me to the shop"]
        );
        let bare = target_point(&json!({"covered": true})).unwrap_err();
        assert!(
            bare.downcast_ref::<Covered>()
                .unwrap()
                .controls()
                .is_empty()
        );
    }

    #[test]
    fn the_press_recheck_ignores_the_text_around_the_control() {
        let before = json!([7, "button", "Buy", null, false, "Ticks: 1"]);
        let ticked = json!([7, "button", "Buy", null, false, "Ticks: 2"]);
        let expanded = json!([7, "button", "Buy", null, true, "Ticks: 2"]);
        assert_eq!(control_state(&before), control_state(&ticked));
        assert_ne!(control_state(&before), control_state(&expanded));
        // A control that is gone or hidden has no guard at all.
        assert_eq!(control_state(&Value::Null), Value::Null);
    }

    #[test]
    fn points_within_half_a_pixel_are_the_same() {
        let at = |x, y| Point { x, y };
        assert!(at(10.0, 10.0).near(at(10.4, 9.6)));
        assert!(!at(10.0, 10.0).near(at(10.0, 10.6)));
    }

    #[test]
    fn a_select_the_page_put_back_is_refused_not_an_error() {
        let action = json!({"label": "Size → Large", "value": "l"});
        assert_eq!(
            select_outcome(&action, &json!({"kept": true, "shown": "Large"})).unwrap(),
            JevActOutcome::done()
        );
        assert_eq!(
            select_outcome(&action, &json!({"kept": false, "shown": "Medium"}))
                .unwrap()
                .refused
                .as_deref(),
            Some("The page kept \"Medium\" instead of \"Large\".")
        );
        assert!(select_outcome(&action, &Value::Null).is_err());
    }
}
