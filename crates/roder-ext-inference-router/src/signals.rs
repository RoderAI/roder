use roder_api::inference_routing::InferenceRoutingSignal;

const SIGNAL_SOURCE: &str = "local_profiler";

const RISK_KEYWORDS: &[(&str, &[&str])] = &[
    (
        "security",
        &[
            "auth",
            "oauth",
            "permission",
            "secret",
            "credential",
            "security",
            "vulnerability",
            "encryption",
            "sql injection",
            "xss",
            "csrf",
        ],
    ),
    (
        "data_loss",
        &[
            "delete",
            "drop table",
            "migration",
            "destructive",
            "overwrite",
            "data loss",
            "reset --hard",
            "rm -rf",
        ],
    ),
    (
        "infra",
        &[
            "deploy",
            "production",
            "terraform",
            "kubernetes",
            "database",
            "dns",
            "ssl",
            "github actions",
            "ci",
        ],
    ),
    (
        "architecture",
        &[
            "architecture",
            "protocol",
            "api contract",
            "cross-cutting",
            "large refactor",
            "multi-crate",
            "distributed",
        ],
    ),
    (
        "privacy",
        &[
            "privacy",
            "pii",
            "personal data",
            "gdpr",
            "hipaa",
            "compliance",
        ],
    ),
];

const INTENT_KEYWORDS: &[(&str, &[&str])] = &[
    (
        "file_lookup",
        &["find", "where is", "look up", "read", "inspect", "show me"],
    ),
    (
        "small_edit",
        &["typo", "small fix", "rename", "format", "lint", "copy edit"],
    ),
    (
        "documentation",
        &["docs", "documentation", "readme", "explain", "comment"],
    ),
    (
        "debug",
        &[
            "bug", "failing", "failure", "error", "panic", "trace", "debug",
        ],
    ),
    (
        "testing",
        &["test", "coverage", "assertion", "snapshot", "fixture"],
    ),
    (
        "architecture",
        &["design", "architecture", "plan", "strategy", "tradeoff"],
    ),
];

pub fn text_signals(text: &str) -> Vec<InferenceRoutingSignal> {
    let normalized = text.to_ascii_lowercase();
    let mut signals = Vec::new();
    for (risk, keywords) in RISK_KEYWORDS {
        if keywords.iter().any(|keyword| normalized.contains(keyword)) {
            signals.push(local_signal("risk", *risk, Some(1.0)));
        }
    }
    for (intent, keywords) in INTENT_KEYWORDS {
        if keywords.iter().any(|keyword| normalized.contains(keyword)) {
            signals.push(local_signal("intent", *intent, Some(0.6)));
        }
    }
    signals
}

pub fn local_signal(
    key: impl Into<String>,
    value: impl Into<String>,
    weight: Option<f64>,
) -> InferenceRoutingSignal {
    InferenceRoutingSignal {
        key: key.into(),
        value: value.into(),
        source: Some(SIGNAL_SOURCE.to_string()),
        weight,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_signals_extract_risk_and_intent() {
        let signals = text_signals("Fix failing auth tests around token permissions");

        assert!(
            signals
                .iter()
                .any(|s| s.key == "risk" && s.value == "security")
        );
        assert!(
            signals
                .iter()
                .any(|s| s.key == "intent" && s.value == "debug")
        );
        assert!(
            signals
                .iter()
                .any(|s| s.key == "intent" && s.value == "testing")
        );
    }
}
