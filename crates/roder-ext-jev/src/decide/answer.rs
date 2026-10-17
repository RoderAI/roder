//! Validate model heads and resolve their selected target to an offered action.
use super::reject_refusal;
use crate::space::ActionSpace;
use anyhow::{Context, bail};
use serde_json::{Map, Value};

pub(super) struct Answer {
    pub choice: String,
    pub operation: String,
    pub target: Option<String>,
    pub confidence: f64,
    pub target_confidence: Option<f64>,
    pub probabilities: Map<String, Value>,
}

pub(super) fn read_answer(
    result: &Value,
    space: &ActionSpace,
    operation_ids: &[String],
) -> anyhow::Result<Answer> {
    let answers = &result["answers"];
    let head = &answers["operation"];
    reject_refusal(head, "operation")?;
    validate_choice(head, operation_ids)?;
    let operation = head["choice"].as_str().unwrap();
    let confidence = head["confidence"].as_f64().unwrap();
    if space.targets_for(operation).is_some() {
        return target(answers, space, operation, confidence);
    }
    let choice = space
        .control(operation)
        .and_then(|a| a["id"].as_str())
        .unwrap_or(operation)
        .to_string();
    let probabilities =
        Map::from_iter([(choice.clone(), head["probabilities"][operation].clone())]);
    Ok(Answer {
        choice,
        operation: operation.to_string(),
        target: None,
        confidence,
        target_confidence: None,
        probabilities,
    })
}

/// The operation may be selected by a separate typed composition. Its support
/// is supplied by that policy; the conditional target distribution stays raw.
pub(super) fn target(
    answers: &Value,
    space: &ActionSpace,
    operation: &str,
    confidence: f64,
) -> anyhow::Result<Answer> {
    let candidates = space
        .targets_for(operation)
        .context("Missing target group")?;
    let ids = candidates
        .iter()
        .map(|(index, _)| index.clone())
        .collect::<Vec<_>>();
    let name = format!("{}_target", operation.to_lowercase());
    let head = &answers[&name];
    reject_refusal(head, &name)?;
    validate_choice(head, &ids)?;
    let selected = head["choice"].as_str().unwrap();
    let action = space
        .target_action(operation, selected)
        .context("Decision service chose an unoffered target")?;
    let probabilities = candidates
        .iter()
        .map(|(index, action)| {
            (
                action["id"].as_str().unwrap_or_default().to_string(),
                head["probabilities"][index].clone(),
            )
        })
        .collect();
    Ok(Answer {
        choice: action["id"].as_str().unwrap_or_default().to_string(),
        operation: operation.to_string(),
        target: Some(selected.to_string()),
        confidence,
        target_confidence: head["confidence"].as_f64(),
        probabilities,
    })
}

pub(crate) fn validate_choice(answer: &Value, ids: &[String]) -> anyhow::Result<()> {
    let Some(probabilities) = answer.get("probabilities").and_then(Value::as_object) else {
        bail!("Invalid browser decision response; no action executed.");
    };
    let Some(choice) = answer.get("choice").and_then(Value::as_str) else {
        bail!("Invalid browser decision response; no action executed.");
    };
    let Some(confidence) = answer.get("confidence").and_then(Value::as_f64) else {
        bail!("Invalid browser decision response; no action executed.");
    };
    let known = ids.iter().any(|id| id == choice)
        && probabilities.len() == ids.len()
        && ids.iter().all(|id| probabilities.contains_key(id));
    let mut numbers = Vec::with_capacity(probabilities.len() + 1);
    for value in probabilities.values() {
        match value.as_f64() {
            Some(number) => numbers.push(number),
            None => bail!("Invalid browser decision response; no action executed."),
        }
    }
    numbers.push(confidence);
    let bounded = numbers
        .iter()
        .all(|number| number.is_finite() && (0.0..=1.0).contains(number));
    let total = probabilities
        .values()
        .filter_map(Value::as_f64)
        .sum::<f64>();
    let highest = probabilities
        .values()
        .filter_map(Value::as_f64)
        .fold(f64::NEG_INFINITY, f64::max);
    let chosen = probabilities
        .get(choice)
        .and_then(Value::as_f64)
        .unwrap_or(f64::NEG_INFINITY);
    if !(known && bounded && (total - 1.0).abs() < 0.02 && chosen >= highest - 1e-6) {
        bail!("Invalid browser decision response; no action executed.");
    }
    Ok(())
}
