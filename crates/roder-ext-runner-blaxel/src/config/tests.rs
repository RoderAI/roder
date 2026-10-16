use serde_json::json;

use super::{ExpirationPolicyType, parse_lifecycle, parse_standby_after};

#[test]
fn parses_standby_after_and_explicit_disable() {
    assert_eq!(
        parse_standby_after(&json!({ "standby_after": "5m" })).unwrap(),
        Some(300)
    );
    assert_eq!(
        parse_standby_after(&json!({ "standby_after": "300s" })).unwrap(),
        Some(300)
    );
    assert_eq!(
        parse_standby_after(&json!({ "standby_after": "0s" })).unwrap(),
        None
    );
}

#[test]
fn standby_after_is_strict_and_bounded() {
    for invalid in [json!(300), json!("5 minutes"), json!("1.5h"), json!("25h")] {
        let error = parse_standby_after(&json!({ "standby_after": invalid })).unwrap_err();
        assert!(error.to_string().contains("standby_after"));
    }
}

#[test]
fn parses_lifecycle_policy_and_emits_blaxel_shape() {
    let lifecycle = parse_lifecycle(&json!({
        "lifecycle": {
            "expiration_policies": [{ "type": "ttl-idle", "value": "7d" }]
        }
    }))
    .unwrap()
    .unwrap();

    assert_eq!(lifecycle.expiration_policies.len(), 1);
    assert_eq!(
        lifecycle.expiration_policies[0].kind,
        ExpirationPolicyType::TtlIdle
    );
    assert_eq!(
        lifecycle.api_value(),
        json!({
            "expirationPolicies": [{
                "action": "delete",
                "type": "ttl-idle",
                "value": "7d"
            }]
        })
    );
    assert_eq!(
        lifecycle.config_value(),
        json!({
            "expiration_policies": [{ "type": "ttl-idle", "value": "7d" }]
        })
    );
}

#[test]
fn lifecycle_rejects_unknown_keys_actions_and_duplicate_types() {
    let invalid = [
        json!({ "lifecycle": { "unknown": true } }),
        json!({
            "lifecycle": {
                "expiration_policies": [{
                    "type": "ttl-idle",
                    "value": "7d",
                    "unknown": true
                }]
            }
        }),
        json!({
            "lifecycle": {
                "expiration_policies": [{
                    "type": "ttl-idle",
                    "value": "7d",
                    "action": "pause"
                }]
            }
        }),
        json!({
            "lifecycle": {
                "expiration_policies": [
                    { "type": "ttl-idle", "value": "7d" },
                    { "type": "ttl-idle", "value": "14d" }
                ]
            }
        }),
    ];

    for config in invalid {
        assert!(parse_lifecycle(&config).is_err(), "accepted {config}");
    }
}

#[test]
fn lifecycle_empty_policy_list_is_an_explicit_clear() {
    let lifecycle = parse_lifecycle(&json!({
        "lifecycle": { "expiration_policies": [] }
    }))
    .unwrap()
    .unwrap();
    assert_eq!(lifecycle.api_value(), json!({ "expirationPolicies": [] }));
    assert!(parse_lifecycle(&json!({})).unwrap().is_none());
}
