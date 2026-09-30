//! Resolve stable element refs and viewport coordinates before dispatching input.
use super::look::helper;
use super::session::DirectSession;
use anyhow::{Context, bail};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy)]
pub(crate) struct Point {
    pub(crate) x: f64,
    pub(crate) y: f64,
}

impl DirectSession {
    /// Translate a selector or a visible label into the same stable refs used by look.
    /// Ambiguous or missing targets fail before any input is dispatched.
    pub(crate) async fn resolve_ref(
        &mut self,
        selector: &str,
        text: &str,
    ) -> anyhow::Result<String> {
        let resolved = helper(
            &mut self.client,
            &format!("resolve({}, {})", json!(selector), json!(text)),
        )
        .await?;
        if let Some(error) = resolved["error"].as_str() {
            bail!("{error}");
        }
        resolved["ref"]
            .as_str()
            .map(str::to_string)
            .context("No visible target; look again")
    }

    /// The point `args` names under `prefix`: a ref's (centre, or `fx`/`fy`
    /// within its box), or `x`/`y`. `Err` explains what is wrong.
    pub(crate) async fn point(
        &mut self,
        args: &Value,
        prefix: &str,
    ) -> anyhow::Result<Result<Point, String>> {
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
        match (at("x"), at("y")) {
            (Some(x), Some(y)) if x.is_finite() && y.is_finite() => {
                let size = self
                    .client
                    .evaluate_isolated("[innerWidth, innerHeight]")
                    .await?;
                if x < 0.0
                    || y < 0.0
                    || x >= size[0].as_f64().unwrap_or(0.0)
                    || y >= size[1].as_f64().unwrap_or(0.0)
                {
                    return Ok(Err(
                        "Coordinates must be inside the viewport; look or screenshot again".into(),
                    ));
                }
                Ok(Ok(Point { x, y }))
            }
            _ => Ok(Err(format!(
                "give {prefix}ref (from the last look) or {prefix}x and {prefix}y (viewport px)"
            ))),
        }
    }

    /// What sits at the point: the ref's element when a ref was given, else
    /// whatever the point hits.
    pub(crate) async fn probe(
        &mut self,
        args: &Value,
        prefix: &str,
        point: Point,
    ) -> anyhow::Result<Value> {
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
}
