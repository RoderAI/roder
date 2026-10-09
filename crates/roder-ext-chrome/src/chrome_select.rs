//! `chrome_select` arguments, checked before a call goes to either backend.
//!
//! The call is `{tabId?, ref?, selector?, value}`: `value` and exactly one of
//! `ref` and `selector`. The paired extension takes a CSS selector and an
//! exact option value, and its wire command stays `{selector, value}`; the
//! Roder Desktop browser also takes a snapshot ref and matches the option's
//! visible text when no option has that value.

use serde_json::Value;

/// What a `chrome_select` call points at.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Target<'a> {
    /// A ref from the last Desktop snapshot.
    Ref(&'a str),
    /// A CSS selector.
    Selector(&'a str),
}

/// A checked `chrome_select` call.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Select<'a> {
    pub(crate) target: Target<'a>,
    pub(crate) value: &'a str,
}

/// A text argument: absent, null and empty all mean it was not given.
fn text<'a>(arguments: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.as_str()).filter(|text| !text.is_empty())),
        Some(_) => Err(format!("chrome_select: {key} must be a string.")),
    }
}

/// The call's target and value, or what is wrong with it.
pub(crate) fn parse(arguments: &Value) -> Result<Select<'_>, String> {
    let value = match arguments.get("value") {
        Some(Value::String(value)) => value.as_str(),
        _ => {
            return Err(
                "chrome_select needs value: the option's value or its visible text.".to_string(),
            );
        }
    };
    let target = match (text(arguments, "ref")?, text(arguments, "selector")?) {
        (Some(reference), None) => Target::Ref(reference),
        (None, Some(selector)) => Target::Selector(selector),
        (Some(_), Some(_)) => {
            return Err("chrome_select takes ref or selector, not both.".to_string());
        }
        (None, None) => {
            return Err(
                "chrome_select needs a target: pass ref (from chrome_page_snapshot) or selector \
                 (CSS)."
                    .to_string(),
            );
        }
    };
    Ok(Select { target, value })
}

/// Check a call before it is dispatched. With the paired extension (which
/// takes a selector only) a ref cannot be sent on; the call is left exactly
/// as the model made it otherwise, so the extension's command is unchanged.
pub(crate) fn check(arguments: &Value, extension: bool) -> Result<(), String> {
    match parse(arguments)?.target {
        Target::Ref(_) if extension => Err(
            "chrome_select with ref works only on the Roder Desktop browser; the paired Chrome \
             extension needs a CSS selector."
                .to_string(),
        ),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use roder_api::chrome::{ChromeBridge, ChromeController, ChromePermissionMode};
    use roder_api::tools::{
        ToolCall, ToolContributor, ToolExecutionContext, ToolRegistry, ToolResult,
    };
    use serde_json::json;

    use super::*;
    use crate::ChromeToolContributor;

    #[test]
    fn a_call_names_one_target_and_a_value() {
        let by_ref = json!({"ref": "e1-2", "value": "growth"});
        assert_eq!(
            parse(&by_ref).unwrap(),
            Select {
                target: Target::Ref("e1-2"),
                value: "growth"
            }
        );
        let by_selector = json!({"tabId": 4, "selector": "#plan", "value": ""});
        assert_eq!(
            parse(&by_selector).unwrap(),
            Select {
                target: Target::Selector("#plan"),
                value: ""
            },
            "an empty value is the value of a placeholder option"
        );
        // A null or empty target is one that was not given.
        let blank = json!({"ref": null, "selector": "#plan", "value": "x"});
        assert_eq!(parse(&blank).unwrap().target, Target::Selector("#plan"));
        let blank = json!({"ref": "", "selector": "#plan", "value": "x"});
        assert_eq!(parse(&blank).unwrap().target, Target::Selector("#plan"));
    }

    #[test]
    fn a_call_without_exactly_one_target_and_a_value_says_what_is_missing() {
        for (arguments, wanted) in [
            (json!({"value": "x"}), "needs a target"),
            (
                json!({"ref": "", "selector": null, "value": "x"}),
                "needs a target",
            ),
            (
                json!({"ref": "e1-2", "selector": "#plan", "value": "x"}),
                "not both",
            ),
            (json!({"selector": "#plan"}), "needs value"),
            (json!({"selector": "#plan", "value": null}), "needs value"),
            (json!({"selector": "#plan", "value": 3}), "needs value"),
            (json!({"ref": 7, "value": "x"}), "ref must be a string"),
        ] {
            let error = parse(&arguments).unwrap_err();
            assert!(error.contains(wanted), "{arguments}: {error}");
            assert_eq!(check(&arguments, false).unwrap_err(), error);
            assert_eq!(check(&arguments, true).unwrap_err(), error);
        }
    }

    #[test]
    fn only_the_extension_refuses_a_ref() {
        let by_ref = json!({"ref": "e1-2", "value": "x"});
        assert!(check(&by_ref, false).is_ok());
        assert!(check(&by_ref, true).unwrap_err().contains("selector"));
        assert!(check(&json!({"selector": "#plan", "value": "x"}), true).is_ok());
    }

    async fn run(bridge: Arc<ChromeBridge>, arguments: Value) -> ToolResult {
        let mut registry = ToolRegistry::default();
        ChromeToolContributor::with_controller(bridge)
            .contribute(&mut registry)
            .unwrap();
        registry
            .get("chrome_select")
            .expect("chrome_select is registered")
            .execute(
                ToolExecutionContext::new(
                    "thread",
                    "turn",
                    roder_api::policy_mode::PolicyMode::Default,
                ),
                ToolCall {
                    id: "call-1".to_string(),
                    name: "chrome_select".to_string(),
                    raw_arguments: arguments.to_string(),
                    arguments,
                    thread_id: "thread".to_string(),
                    turn_id: "turn".to_string(),
                },
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn the_extension_still_receives_selector_and_value_only_as_given() {
        let bridge = Arc::new(ChromeBridge::new());
        bridge.set_enabled(true);
        bridge.set_mode(ChromePermissionMode::Control);
        let mut registration = bridge.register_client(None, &json!({ "capabilities": [] }));
        let echo = bridge.clone();
        let extension = tokio::spawn(async move {
            let frame = registration.commands.recv().await.unwrap();
            let id = frame["id"].as_str().unwrap().to_string();
            echo.ingest_frame(
                Some(registration.client_id),
                json!({ "type": "command/result", "id": id, "ok": true, "result": { "value": "pro" } }),
            );
            frame
        });
        let result = run(
            bridge,
            json!({"tabId": 4, "selector": "#plan", "value": "pro"}),
        )
        .await;
        let frame = extension.await.unwrap();
        assert!(!result.is_error, "{}", result.text);
        assert_eq!(frame["type"], "page/select");
        assert_eq!(frame["tabId"], 4);
        assert_eq!(frame["selector"], "#plan");
        assert_eq!(frame["value"], "pro");
        let mut keys = frame
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        keys.sort_unstable();
        assert_eq!(keys, ["id", "selector", "tabId", "type", "value"]);
    }

    #[tokio::test]
    async fn a_ref_is_refused_before_it_reaches_the_extension() {
        let bridge = Arc::new(ChromeBridge::new());
        bridge.set_enabled(true);
        bridge.set_mode(ChromePermissionMode::Control);
        let mut registration = bridge.register_client(None, &json!({ "capabilities": [] }));
        let result = run(bridge, json!({"ref": "e1-2", "value": "pro"})).await;
        assert!(
            result.is_error && result.text.contains("CSS selector"),
            "{}",
            result.text
        );
        assert!(
            registration.commands.try_recv().is_err(),
            "the extension must not get a command it cannot read"
        );
    }
}
