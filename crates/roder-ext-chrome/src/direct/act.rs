//! The actions: real DevTools input at a ref's point or at coordinates.
//!
//! A ref from the last look is resolved to the centre of its box (scrolled
//! into view first) and hit-tested: a ref another element covers is not
//! pressed, and the result names what covers it, so the caller can close it
//! or press at coordinates deliberately. Coordinates are pressed as given.
//! Before a click on a control, or Enter, the owner's guard is asked whether
//! the control needs the user's confirmation first.

use anyhow::{Context, bail};
use serde_json::{Value, json};

use super::client::cut;
use super::guard::{GateAction, GateQuery};
use super::keys::Chord;
use super::look::helper;
use super::session::{DirectSession, DirectStep};

/// Why the guard held a press back.
enum Held {
    /// Never pressed by these tools; the caller may try something else.
    Refused(String),
    /// Needs the user's confirmation; the run stops.
    Confirm(String),
}

/// A point in viewport CSS pixels.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Point {
    x: f64,
    y: f64,
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
        let point = match self.point(args, "").await? {
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
        let point = match self.point(args, "").await? {
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
        let from = match self.point(args, "from_").await? {
            Ok(point) => point,
            Err(why) => return Ok(DirectStep::error(why)),
        };
        let from_probe = self.probe(args, "from_", from).await?;
        let to = match self.point(args, "to_").await? {
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

    async fn type_text(&mut self, args: &Value) -> anyhow::Result<DirectStep> {
        let Some(text) = args["text"].as_str() else {
            return Ok(DirectStep::error("type needs text"));
        };
        let target = args["ref"]
            .as_str()
            .filter(|reference| !reference.is_empty());
        // A ref's content is replaced unless the call appends to it.
        let clear = target.is_some() && args["append"] != json!(true);
        if let Some(reference) = target {
            let point = match self.point(args, "").await? {
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
        let into = helper(&mut self.client, "probeFocused()").await?;
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

    async fn key(&mut self, args: &Value) -> anyhow::Result<DirectStep> {
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
        let focused = helper(&mut self.client, "probeFocused()").await?;
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
            || args["x"].as_f64().is_some_and(|x| x != 0.0)
            || args["y"].as_f64().is_some_and(|y| y != 0.0);
        let point = match located {
            true => match self.point(args, "").await? {
                Ok(point) => point,
                Err(why) => return Ok(DirectStep::error(why)),
            },
            false => {
                let size = self.client.evaluate("[innerWidth, innerHeight]").await?;
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

    async fn select(&mut self, args: &Value) -> anyhow::Result<DirectStep> {
        let (Some(reference), Some(option)) = (args["ref"].as_str(), args["option"].as_str())
        else {
            return Ok(DirectStep::error("select needs a ref and an option"));
        };
        let chosen = helper(
            &mut self.client,
            &format!("select({}, {})", json!(reference), json!(option)),
        )
        .await?;
        if chosen["gone"] == json!(true) {
            return Ok(DirectStep::error(format!(
                "{reference} is gone from the page; look again"
            )));
        }
        if chosen["not_select"] == json!(true) {
            return Ok(DirectStep::error(format!(
                "{reference} is not a native select; click it and then the option instead"
            )));
        }
        if chosen["missing"] == json!(true) {
            return Ok(DirectStep::error(format!(
                "{reference} has no option {option:?}; its options: {}",
                chosen["options"]
            )));
        }
        let shown = chosen["shown"].as_str().unwrap_or_default().to_string();
        let done = match chosen["kept"] == json!(true) {
            true => format!("Chose \"{shown}\" in {reference}."),
            false => format!("Chose {option:?} in {reference}, but the page shows \"{shown}\"."),
        };
        self.after(
            done,
            json!({"ref": reference, "option": option, "shown": shown}),
        )
        .await
    }

    /// The point `args` names under `prefix`: a ref's (centre, or `fx`/`fy`
    /// within its box), or `x`/`y`. `Err` explains what is wrong.
    async fn point(&mut self, args: &Value, prefix: &str) -> anyhow::Result<Result<Point, String>> {
        let at = |key: &str| args[format!("{prefix}{key}")].as_f64();
        if let Some(reference) = args[format!("{prefix}ref")]
            .as_str()
            .filter(|reference| !reference.is_empty())
        {
            let resolved = helper(
                &mut self.client,
                &format!(
                    "point({}, {}, {})",
                    json!(reference),
                    at("fx").map_or(json!(null), |fx| json!(fx.clamp(0.0, 1.0))),
                    at("fy").map_or(json!(null), |fy| json!(fy.clamp(0.0, 1.0))),
                ),
            )
            .await?;
            if resolved["gone"] == json!(true) {
                return Ok(Err(format!(
                    "{reference} is not on the page any more (or not shown); look again for \
                     current refs"
                )));
            }
            let point = Point {
                x: resolved["x"].as_f64().context("a point")?,
                y: resolved["y"].as_f64().context("a point")?,
            };
            if resolved["covered"] == json!(true) {
                return Ok(Err(format!(
                    "{reference} is covered at ({:.0},{:.0}) by {}; nothing was pressed. Close \
                     what covers it (Escape, or its close control) and try again, or press at \
                     x/y if you mean to press what is on top.",
                    point.x,
                    point.y,
                    resolved["by"]
                        .as_str()
                        .map_or("another element".to_string(), |by| format!("\"{by}\""))
                )));
            }
            return Ok(Ok(point));
        }
        // Roder sends every property, so 0,0 is how a call leaves x/y out.
        match (at("x"), at("y")) {
            (Some(x), Some(y)) if x != 0.0 || y != 0.0 => Ok(Ok(Point { x, y })),
            _ => Ok(Err(format!(
                "give {prefix}ref (from the last look) or {prefix}x and {prefix}y (viewport px)"
            ))),
        }
    }

    /// What sits at the point: the ref's element when a ref was given, else
    /// whatever the point hits.
    async fn probe(&mut self, args: &Value, prefix: &str, point: Point) -> anyhow::Result<Value> {
        let call = match args[format!("{prefix}ref")]
            .as_str()
            .filter(|reference| !reference.is_empty())
        {
            Some(reference) => format!("probeRef({})", json!(reference)),
            None => format!("probe({}, {})", point.x, point.y),
        };
        let mut probe = helper(&mut self.client, &call).await?;
        if let Some(label) = probe["label"].as_str() {
            probe["label"] = json!(self.guard.scrub(label));
        }
        Ok(probe)
    }

    /// Whether the guard holds back this press: a control the owner never
    /// lets the tools press (an error the caller can work around), or one
    /// that needs the user's confirmation, unless the call is authorized and
    /// the host may authorize it (which stops the run).
    fn gate(&self, action: GateAction, probe: &Value, args: &Value) -> Option<Held> {
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
        Ok(covered.as_str().map(|by| {
            DirectStep::error(format!(
                "{} is covered by \"{by}\"; pressing it by keyboard would get around what covers \
                 it, so nothing was pressed. Close what covers it first (Escape, or its close \
                 control), or scroll it clear.",
                named(focused)
            ))
        }))
    }

    /// The step a held press comes to, after `done` (what already ran).
    fn held(&self, held: Held, done: String) -> DirectStep {
        match held {
            Held::Refused(reason) => DirectStep::error(format!("{done}{reason}")),
            Held::Confirm(reason) => self.confirm_first(format!("{done}{reason}")),
        }
    }

    async fn press(&mut self, chord: &Chord) -> anyhow::Result<()> {
        for event in chord.events() {
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
            params["buttons"] = json!(if kind == "mousePressed" { 1 } else { 0 });
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

/// A probe as the result names it: `button "Search"`, or `the page`.
pub(crate) fn named(probe: &Value) -> String {
    let what = probe["role"]
        .as_str()
        .or_else(|| probe["tag"].as_str())
        .unwrap_or("the page");
    match probe["label"]
        .as_str()
        .filter(|label| !label.trim().is_empty())
    {
        Some(label) => format!("{what} \"{}\"", cut(label, 60)),
        None => what.to_string(),
    }
}
