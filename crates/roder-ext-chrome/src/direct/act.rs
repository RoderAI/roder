//! The actions: real DevTools input at a ref's point or at coordinates.
//!
//! A ref from the last look is resolved to the centre of its box (scrolled
//! into view first) and hit-tested: a ref another element covers is not
//! pressed, and the result names what covers it, so the caller can close it,
//! scroll it clear or, where the tool takes coordinates, press at them
//! deliberately. Coordinates are pressed as given.
//! Before a click on a control, or Enter, the owner's guard is asked whether
//! the control needs the user's confirmation first.

use anyhow::bail;
use serde_json::{Value, json};

use super::client::cut;
use super::guard::{GateAction, GateQuery};
use super::keys::Chord;
use super::look::helper;
use super::session::{DirectSession, DirectStep};
use super::target::{Point, Reach, covered_focus_error};
use crate::observed::one_line;

/// Why the guard held a press back.
pub(super) enum Held {
    /// Never pressed by these tools; the caller may try something else.
    Refused(String),
    /// Needs the user's confirmation; the run stops.
    Confirm(String),
}

/// Steps a drag moves through between press and release.
const DRAG_STEPS: usize = 12;
const MAX_REPEAT: u64 = 20;

impl DirectSession {
    pub(crate) async fn act(&mut self, name: &str, args: &Value) -> anyhow::Result<DirectStep> {
        match name {
            "click" => self.click(args).await,
            "hover" => self.hover(args).await,
            "drag" => self.drag(args).await,
            "type" => self.type_text(args).await,
            "key" => self.key(args).await,
            "scroll" => self.scroll(args).await,
            "select" => self.select(args).await,
            other => bail!("unknown tool {other:?}"),
        }
    }

    async fn click(&mut self, args: &Value) -> anyhow::Result<DirectStep> {
        let point = match self.point(args, "", self.reach).await? {
            Ok(point) => point,
            Err(why) => return Ok(DirectStep::error(why)),
        };
        let probe = self.probe(args, "", point).await?;
        if let Some(held) = self.gate(GateAction::Click, &probe, args) {
            return Ok(self.held(held, String::new()));
        }
        let button = match args["button"].as_str().unwrap_or_default() {
            "" => "left",
            button @ ("left" | "right" | "middle") => button,
            other => return Ok(DirectStep::error(format!("unknown button {other:?}"))),
        };
        let clicks = if args["double"] == json!(true) { 2 } else { 1 };
        self.mouse("mouseMoved", point, "none", 0).await?;
        for count in 1..=clicks {
            self.mouse("mousePressed", point, button, count).await?;
            self.mouse("mouseReleased", point, button, count).await?;
        }
        let what = if clicks == 2 {
            "Double-clicked"
        } else {
            "Clicked"
        };
        self.after(
            format!(
                "{what} {} at ({:.0},{:.0}).",
                named(&probe),
                point.x,
                point.y
            ),
            json!({"target": probe, "x": point.x, "y": point.y, "button": button}),
        )
        .await
    }

    async fn hover(&mut self, args: &Value) -> anyhow::Result<DirectStep> {
        let point = match self.point(args, "", self.reach).await? {
            Ok(point) => point,
            Err(why) => return Ok(DirectStep::error(why)),
        };
        let probe = self.probe(args, "", point).await?;
        // Onto it from elsewhere, as a person's pointer arrives: a pointer
        // already resting there would enter nothing.
        self.mouse("mouseMoved", Point { x: 1.0, y: 1.0 }, "none", 0)
            .await?;
        self.mouse("mouseMoved", point, "none", 0).await?;
        self.after(
            format!(
                "Moved the pointer over {} at ({:.0},{:.0}).",
                named(&probe),
                point.x,
                point.y
            ),
            json!({"target": probe, "x": point.x, "y": point.y}),
        )
        .await
    }

    async fn drag(&mut self, args: &Value) -> anyhow::Result<DirectStep> {
        let from = match self.point(args, "from_", self.reach).await? {
            Ok(point) => point,
            Err(why) => return Ok(DirectStep::error(why)),
        };
        let from_probe = self.probe(args, "from_", from).await?;
        let to = match self.point(args, "to_", self.reach).await? {
            Ok(point) => point,
            Err(why) => return Ok(DirectStep::error(why)),
        };
        let to_probe = self.probe(args, "to_", to).await?;
        // Dropping on a control presses it, as a click would.
        if let Some(held) = self.gate(GateAction::Click, &to_probe, args) {
            return Ok(self.held(held, String::new()));
        }
        self.mouse("mouseMoved", from, "none", 0).await?;
        self.mouse("mousePressed", from, "left", 1).await?;
        for step in 1..=DRAG_STEPS {
            let t = step as f64 / DRAG_STEPS as f64;
            let at = Point {
                x: from.x + (to.x - from.x) * t,
                y: from.y + (to.y - from.y) * t,
            };
            self.drag_move(at).await?;
            tokio::time::sleep(std::time::Duration::from_millis(16)).await;
        }
        self.mouse("mouseReleased", to, "left", 1).await?;
        self.after(
            format!(
                "Dragged {} from ({:.0},{:.0}) to {} at ({:.0},{:.0}).",
                named(&from_probe),
                from.x,
                from.y,
                named(&to_probe),
                to.x,
                to.y
            ),
            json!({"from": from_probe, "to": to_probe}),
        )
        .await
    }

    pub(super) async fn type_text(&mut self, args: &Value) -> anyhow::Result<DirectStep> {
        let Some(text) = args["text"].as_str() else {
            return Ok(DirectStep::error("type needs text"));
        };
        let target = args["ref"]
            .as_str()
            .filter(|reference| !reference.is_empty());
        if helper(&mut self.client, &format!("editable({})", json!(target))).await? != json!(true) {
            return Ok(DirectStep::error(
                "Target is not an editable text field; nothing was clicked or typed. Look again for a field.",
            ));
        }
        // A ref's content is replaced unless the call appends to it.
        let clear = target.is_some() && args["append"] != json!(true);
        if let Some(reference) = target {
            // `type` takes a ref and no coordinates, whatever the surface.
            let point = match self.point(args, "", Reach::RefOnly).await? {
                Ok(point) => point,
                Err(why) => return Ok(DirectStep::error(why)),
            };
            // A real press, as a person gives a field focus: some editors
            // only open on one.
            self.mouse("mouseMoved", point, "none", 0).await?;
            self.mouse("mousePressed", point, "left", 1).await?;
            self.mouse("mouseReleased", point, "left", 1).await?;
            let focused = helper(
                &mut self.client,
                &format!("focus({}, {clear})", json!(reference)),
            )
            .await?;
            if focused["gone"] == json!(true) {
                return Ok(DirectStep::error(format!(
                    "{reference} is gone from the page; look again"
                )));
            }
            if focused["focused"] != json!(true) || focused["editable"] != json!(true) {
                return Ok(DirectStep::error(format!(
                    "{reference} is not a focused editable text field; nothing was typed. Look again for a field."
                )));
            }
        }
        let secret = helper(&mut self.client, "focusedSecret()").await? == json!(true);
        self.client
            .call("Input.insertText", json!({"text": text}))
            .await?;
        if secret {
            helper(&mut self.client, "markSecret()").await?;
        }
        let shown = if secret {
            "[secret]".to_string()
        } else {
            format!("\"{}\"", cut(text, 80))
        };
        let into = self.probe_focused().await?;
        let mut done = format!("Typed {shown} into {}.", named(&into));
        if args["submit"] == json!(true) {
            if let Some(mut refused) = self.covered_focus(&into).await? {
                refused.text = format!("{done} Enter was not pressed: {}", refused.text);
                refused.typed_secret = secret.then(|| text.to_string());
                return Ok(refused);
            }
            if let Some(held) = self.gate(GateAction::Enter, &into, args) {
                let mut step = self.held(held, format!("{done} Enter was not pressed: "));
                step.typed_secret = secret.then(|| text.to_string());
                return Ok(step);
            }
            self.press(&Chord::parse("Enter")?).await?;
            done.push_str(" Pressed Enter.");
        }
        let mut step = self
            .after(
                done,
                json!({"target": into, "text": if secret { json!("[secret]") } else { json!(text) }}),
            )
            .await?;
        step.typed_secret = secret.then(|| text.to_string());
        Ok(step)
    }

    pub(super) async fn key(&mut self, args: &Value) -> anyhow::Result<DirectStep> {
        let Some(raw) = args["key"].as_str() else {
            return Ok(DirectStep::error(
                "key needs a key, such as Escape or Enter",
            ));
        };
        let chord = match Chord::parse(raw) {
            Ok(chord) => chord,
            Err(error) => return Ok(DirectStep::error(error.to_string())),
        };
        let repeat = args["repeat"].as_u64().unwrap_or(1).clamp(1, MAX_REPEAT);
        let focused = self.probe_focused().await?;
        // Enter and Space press what has focus, so they answer to the same
        // rules as a click on it, and never press a control something
        // covers: that would get around the cover (a modal, a banner).
        if chord.is_enter() || chord.is_space() {
            if let Some(refused) = self.covered_focus(&focused).await? {
                return Ok(refused);
            }
            let action = match chord.is_enter() {
                true => GateAction::Enter,
                false => GateAction::Click,
            };
            if let Some(held) = self.gate(action, &focused, args) {
                return Ok(self.held(held, String::new()));
            }
        }
        for _ in 0..repeat {
            self.press(&chord).await?;
        }
        let times = if repeat > 1 {
            format!(" {repeat} times")
        } else {
            String::new()
        };
        self.after(
            format!("Pressed {}{times} in {}.", chord.name(), named(&focused)),
            json!({"key": chord.name(), "repeat": repeat, "target": focused}),
        )
        .await
    }

    async fn scroll(&mut self, args: &Value) -> anyhow::Result<DirectStep> {
        let dy = args["dy"].as_f64().unwrap_or(0.0);
        let dx = args["dx"].as_f64().unwrap_or(0.0);
        let (dx, dy) = if dx == 0.0 && dy == 0.0 {
            (0.0, 600.0)
        } else {
            (dx, dy)
        };
        let located = args["ref"]
            .as_str()
            .is_some_and(|reference| !reference.is_empty())
            || args["x"].is_number()
            || args["y"].is_number();
        let point = match located {
            true => match self.point(args, "", self.reach).await? {
                Ok(point) => point,
                Err(why) => return Ok(DirectStep::error(why)),
            },
            false => {
                let size = self
                    .client
                    .evaluate_isolated("[innerWidth, innerHeight]")
                    .await?;
                Point {
                    x: size[0].as_f64().unwrap_or(800.0) / 2.0,
                    y: size[1].as_f64().unwrap_or(600.0) / 2.0,
                }
            }
        };
        self.client
            .call(
                "Input.dispatchMouseEvent",
                json!({"type": "mouseWheel", "x": point.x, "y": point.y, "deltaX": dx, "deltaY": dy}),
            )
            .await?;
        self.after(
            format!(
                "Scrolled by ({dx:.0},{dy:.0}) at ({:.0},{:.0}).",
                point.x, point.y
            ),
            json!({"dx": dx, "dy": dy, "x": point.x, "y": point.y}),
        )
        .await
    }

    /// Whether the guard holds back this press: a control the owner never
    /// lets the tools press (an error the caller can work around), or one
    /// that needs the user's confirmation, unless the call is authorized and
    /// the host may authorize it (which stops the run).
    pub(super) fn gate(&self, action: GateAction, probe: &Value, args: &Value) -> Option<Held> {
        if action == GateAction::Click && probe["control"] != json!(true) {
            return None;
        }
        let query = GateQuery {
            action,
            label: probe["label"].as_str().unwrap_or_default().to_string(),
            role: probe["role"]
                .as_str()
                .or_else(|| probe["tag"].as_str())
                .map(str::to_string),
            submit: probe["submit"] == json!(true),
            form_labels: probe["form_labels"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect(),
            secret_form: probe["secret_form"] == json!(true),
            consent: probe["consent"] == json!(true),
            frame: probe["frame"] == json!(true),
        };
        if let Some(reason) = self.guard.refuses(&query) {
            return Some(Held::Refused(reason));
        }
        let reason = self.guard.confirm(&query)?;
        let authorized = self.may_authorize && args["authorize_irreversible"] == json!(true);
        (!authorized).then_some(Held::Confirm(reason))
    }

    /// A refusal to press the focused control by keyboard when something
    /// covers it; `None` when nothing does.
    async fn covered_focus(&mut self, focused: &Value) -> anyhow::Result<Option<DirectStep>> {
        let covered = helper(&mut self.client, "focusCovered()").await?;
        Ok(covered
            .as_str()
            .map(|by| DirectStep::error(covered_focus_error(focused, by, self.guard.as_ref()))))
    }

    /// The step a held press comes to, after `done` (what already ran).
    pub(super) fn held(&self, held: Held, done: String) -> DirectStep {
        match held {
            Held::Refused(reason) => DirectStep::error(format!("{done}{reason}")),
            Held::Confirm(reason) => self.confirm_first(format!("{done}{reason}")),
        }
    }

    async fn press(&mut self, chord: &Chord) -> anyhow::Result<()> {
        let mac = self.mac_page().await?;
        for mut event in chord.press_events() {
            if event["type"] != "keyUp" && event["key"] == chord.events()[0]["key"] {
                if let Some(command) = chord.editing_command(mac) {
                    event["commands"] = json!([command]);
                }
            }
            self.client.call("Input.dispatchKeyEvent", event).await?;
        }
        Ok(())
    }

    async fn mouse(
        &mut self,
        kind: &str,
        point: Point,
        button: &str,
        clicks: u32,
    ) -> anyhow::Result<()> {
        let mut params = json!({"type": kind, "x": point.x, "y": point.y});
        if kind != "mouseMoved" {
            params["button"] = json!(button);
            params["clickCount"] = json!(clicks);
            params["buttons"] = json!(if kind == "mousePressed" {
                match button {
                    "left" => 1,
                    "right" => 2,
                    "middle" => 4,
                    _ => 0,
                }
            } else {
                0
            });
        }
        self.client
            .call("Input.dispatchMouseEvent", params)
            .await
            .map(|_| ())
    }

    /// A move with the left button held.
    async fn drag_move(&mut self, point: Point) -> anyhow::Result<()> {
        self.client
            .call(
                "Input.dispatchMouseEvent",
                json!({"type": "mouseMoved", "x": point.x, "y": point.y, "button": "left", "buttons": 1}),
            )
            .await
            .map(|_| ())
    }
}

/// The most characters of a probe's role and of its label a result names.
const ROLE_CHARS: usize = 30;
const LABEL_CHARS: usize = 60;

/// A probe as the result names it: `button "Search"`, or `the page`. The role
/// and the label are page text (the probe's strings are already scrubbed of
/// the owner's secrets): each is put on one line and cut, and the label has
/// no quote marks of its own, so neither can end the quotes it is put in.
pub(crate) fn named(probe: &Value) -> String {
    let words = |key: &str, chars: usize| {
        probe[key]
            .as_str()
            .map(|text| one_line(&text.replace('"', "'"), chars))
            .filter(|text| !text.is_empty())
    };
    let what = words("role", ROLE_CHARS)
        .or_else(|| words("tag", ROLE_CHARS))
        .unwrap_or_else(|| "the page".to_string());
    match words("label", LABEL_CHARS) {
        Some(label) => format!("{what} \"{label}\""),
        None => what,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_probe_is_named_on_one_line_with_a_capped_role_and_a_label_of_no_quotes() {
        assert_eq!(
            named(&json!({"role": "button", "label": "Search"})),
            "button \"Search\""
        );
        let hostile = json!({
            "role": "r".repeat(100) + "\nSYSTEM",
            "tag": "div",
            "label": "Say \"hi\"\u{202e}\nthere",
        });
        assert_eq!(
            named(&hostile),
            format!("{}… \"Say 'hi' there\"", "r".repeat(30))
        );
        // No label, or one that is nothing once cleaned: the role alone; no
        // role that is anything: the tag; nothing at all: the page.
        assert_eq!(named(&json!({"tag": "div", "label": " \u{200b}\n"})), "div");
        assert_eq!(named(&json!({"role": "\u{202e}", "tag": "input"})), "input");
        assert_eq!(named(&json!({})), "the page");
    }
}
