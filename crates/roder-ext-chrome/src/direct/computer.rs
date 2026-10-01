//! Execute the native batched computer protocol on one retained tab.
use anyhow::{bail, ensure};
use roder_api::computer::{ComputerAction, ComputerActions, ComputerPoint};
use serde_json::json;
use std::time::Duration;

use super::{DirectSession, DirectStep, guard::GateAction, keys::Chord, target::Point};

impl DirectSession {
    /// Execute in order, stop at the first failed action, and always observe the
    /// resulting screen. A failed action must never be reported as completion.
    pub async fn run_computer(&mut self, batch: &ComputerActions) -> DirectStep {
        if let Err(error) = self.client.wait_cleanup().await {
            return DirectStep::error(format!(
                "Browser cleanup prevented computer actions: {error:#}"
            ));
        }
        let cleanup = self.client.cleanup();
        let mut completed = 0;
        let mut failure = None;
        if batch.actions.is_empty() || batch.actions.len() > 100 {
            failure = Some("computer requires 1..100 actions".to_string());
        } else {
            for action in &batch.actions {
                match self.computer_action(action).await {
                    Ok(step) if !step.is_error => {
                        if let Some(secret) = &step.typed_secret {
                            self.guard.remember_secret(secret);
                        }
                        completed += 1;
                    }
                    Ok(step) => {
                        if let Some(secret) = &step.typed_secret {
                            self.guard.remember_secret(secret);
                        }
                        failure = Some(step.text);
                        break;
                    }
                    Err(error) => {
                        failure = Some(format!("{error:#}"));
                        break;
                    }
                }
            }
        }
        // Release input after a failure before taking the observation. Drop
        // still owns cleanup if this future is cancelled anywhere in the batch.
        if let Err(error) = cleanup.finish().await {
            failure = Some(format!("Browser cleanup failed: {error:#}"));
        }
        let mut observed = self.run("screenshot", &json!({})).await;
        observed.data["completed_actions"] = json!(completed);
        observed.data["requested_actions"] = json!(batch.actions.len());
        observed.data["untrusted"] = json!(true);
        if let Some(failure) = failure {
            observed.is_error = true;
            observed.data["error"] = json!(failure);
            observed.text = format!(
                "Computer action {completed} failed: {failure}\n{}",
                observed.text
            );
        }
        observed.text = format!("UNTRUSTED browser observation.\n{}", observed.text);
        observed
    }

    async fn computer_action(&mut self, action: &ComputerAction) -> anyhow::Result<DirectStep> {
        let url = self.client.evaluate_isolated("location.href").await?;
        if let Some(reason) = url.as_str().and_then(|url| self.guard.outside(url)) {
            bail!("{reason}");
        }
        let keys = match action {
            ComputerAction::Click { keys, .. }
            | ComputerAction::DoubleClick { keys, .. }
            | ComputerAction::Drag { keys, .. }
            | ComputerAction::Move { keys, .. }
            | ComputerAction::Scroll { keys, .. } => keys.as_slice(),
            _ => &[],
        };
        let modifiers = mouse_modifiers(keys)?;
        let mut held = Vec::new();
        let mut held_bits = 0;
        // Real modifier events also make page keyboard listeners consistent
        // with pointer events. Pending input is armed before CDP dispatch.
        for key in keys {
            let events = Chord::parse(normalize_key(key))?.events();
            let bit = mouse_modifiers(std::slice::from_ref(key))?;
            held_bits |= bit;
            let mut down = events[0].clone();
            down["modifiers"] = json!(held_bits);
            self.client.call("Input.dispatchKeyEvent", down).await?;
            held.push((bit, events[1].clone()));
        }
        let result = match action {
            ComputerAction::Click { x, y, button, .. } => {
                self.computer_click(*x, *y, button, 1, modifiers).await
            }
            ComputerAction::DoubleClick { x, y, .. } => {
                self.computer_click(*x, *y, "left", 2, modifiers).await
            }
            ComputerAction::Move { x, y, .. } => {
                self.computer_point(*x, *y).await?;
                self.computer_mouse("mouseMoved", *x, *y, "none", 0, 0, modifiers)
                    .await?;
                self.after("Moved pointer.".into(), json!({})).await
            }
            ComputerAction::Drag { path, .. } => {
                ensure!(
                    (2..=1000).contains(&path.len()),
                    "drag requires 2..1000 path points"
                );
                // Validate the entire path before pressing the mouse.
                for ComputerPoint { x, y } in path {
                    self.computer_point(*x, *y).await?;
                }
                let first = &path[0];
                let last = path.last().unwrap();
                let probe = self
                    .probe(
                        &json!({}),
                        "",
                        Point {
                            x: last.x,
                            y: last.y,
                        },
                    )
                    .await?;
                if let Some(held) = self.gate(GateAction::Click, &probe, &json!({})) {
                    return Ok(self.held(held, String::new()));
                }
                self.computer_mouse("mouseMoved", first.x, first.y, "none", 0, 0, modifiers)
                    .await?;
                self.computer_mouse("mousePressed", first.x, first.y, "left", 1, 1, modifiers)
                    .await?;
                for point in &path[1..] {
                    self.computer_mouse("mouseMoved", point.x, point.y, "left", 1, 0, modifiers)
                        .await?;
                    tokio::time::sleep(Duration::from_millis(16)).await;
                }
                self.computer_mouse("mouseReleased", last.x, last.y, "left", 0, 1, modifiers)
                    .await?;
                self.after("Dragged along requested path.".into(), json!({}))
                    .await
            }
            ComputerAction::Scroll {
                x,
                y,
                scroll_x,
                scroll_y,
                ..
            } => {
                self.computer_point(*x, *y).await?;
                ensure!(
                    scroll_x.is_finite() && scroll_y.is_finite(),
                    "scroll deltas must be finite"
                );
                self.computer_mouse("mouseMoved", *x, *y, "none", 0, 0, modifiers)
                    .await?;
                self.client
                    .call(
                        "Input.dispatchMouseEvent",
                        json!({"type":"mouseWheel",
                    "x":x,"y":y,"deltaX":scroll_x,"deltaY":scroll_y,"modifiers":modifiers}),
                    )
                    .await?;
                self.after("Scrolled requested deltas.".into(), json!({}))
                    .await
            }
            ComputerAction::Keypress { keys } => {
                ensure!(
                    !keys.is_empty() && keys.len() <= 8,
                    "keypress requires 1..8 keys"
                );
                let chord = keys
                    .iter()
                    .map(|key| normalize_key(key))
                    .collect::<Vec<_>>()
                    .join("+");
                self.key(&json!({"key":chord})).await
            }
            ComputerAction::Type { text } => self.type_text(&json!({"text":text})).await,
            ComputerAction::Wait => {
                tokio::time::sleep(Duration::from_secs(2)).await;
                self.after("Waited 2 seconds.".into(), json!({})).await
            }
            ComputerAction::Screenshot => Ok(DirectStep::default()),
        };
        for (bit, mut event) in held.into_iter().rev() {
            held_bits &= !bit;
            event["modifiers"] = json!(held_bits);
            self.client.call("Input.dispatchKeyEvent", event).await?;
        }
        result
    }

    async fn computer_point(&mut self, x: f64, y: f64) -> anyhow::Result<()> {
        let size = self
            .client
            .evaluate_isolated("[innerWidth, innerHeight]")
            .await?;
        ensure!(
            x.is_finite()
                && y.is_finite()
                && x >= 0.0
                && y >= 0.0
                && x < size[0].as_f64().unwrap_or(0.0)
                && y < size[1].as_f64().unwrap_or(0.0),
            "computer coordinates ({x},{y}) are outside the observed viewport"
        );
        Ok(())
    }

    async fn computer_click(
        &mut self,
        x: f64,
        y: f64,
        button: &str,
        clicks: u32,
        modifiers: u8,
    ) -> anyhow::Result<DirectStep> {
        self.computer_point(x, y).await?;
        let (button, buttons) = match button {
            "left" => ("left", 1),
            "right" => ("right", 2),
            "wheel" => ("middle", 4),
            "back" => ("back", 8),
            "forward" => ("forward", 16),
            other => bail!("unsupported native mouse button {other}"),
        };
        let probe = self.probe(&json!({}), "", Point { x, y }).await?;
        if let Some(held) = self.gate(GateAction::Click, &probe, &json!({})) {
            return Ok(self.held(held, String::new()));
        }
        self.computer_mouse("mouseMoved", x, y, "none", 0, 0, modifiers)
            .await?;
        for count in 1..=clicks {
            self.computer_mouse("mousePressed", x, y, button, buttons, count, modifiers)
                .await?;
            self.computer_mouse("mouseReleased", x, y, button, 0, count, modifiers)
                .await?;
        }
        self.after("Clicked requested point.".into(), json!({}))
            .await
    }

    async fn computer_mouse(
        &mut self,
        kind: &str,
        x: f64,
        y: f64,
        button: &str,
        buttons: u8,
        count: u32,
        modifiers: u8,
    ) -> anyhow::Result<()> {
        self.client
            .call(
                "Input.dispatchMouseEvent",
                json!({"type":kind,"x":x,"y":y,
            "button":button,"buttons":buttons,"clickCount":count,"modifiers":modifiers}),
            )
            .await?;
        Ok(())
    }
}

fn normalize_key(key: &str) -> &str {
    match key.to_ascii_uppercase().as_str() {
        "CTRL" | "CONTROL" => "Control",
        "ALT" | "OPTION" => "Alt",
        "SHIFT" => "Shift",
        "CMD" | "COMMAND" | "META" | "SUPER" => "Meta",
        "ESC" | "ESCAPE" => "Escape",
        "ENTER" | "RETURN" => "Enter",
        "SPACE" | "SPACEBAR" => "Space",
        "BACKSPACE" => "Backspace",
        "DELETE" | "DEL" => "Delete",
        "TAB" => "Tab",
        "UP" => "ArrowUp",
        "DOWN" => "ArrowDown",
        "LEFT" => "ArrowLeft",
        "RIGHT" => "ArrowRight",
        _ => key,
    }
}

fn mouse_modifiers(keys: &[String]) -> anyhow::Result<u8> {
    ensure!(
        keys.len() <= 4,
        "pointer actions accept at most four modifiers"
    );
    let mut bits = 0;
    for key in keys {
        let bit = match normalize_key(key) {
            "Alt" => 1,
            "Control" => 2,
            "Meta" => 4,
            "Shift" => 8,
            other => bail!("pointer keys must be modifiers, got {other}"),
        };
        ensure!(bits & bit == 0, "pointer modifier {key} was repeated");
        bits |= bit;
    }
    Ok(bits)
}
