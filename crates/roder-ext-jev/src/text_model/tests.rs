use super::*;
use std::collections::HashMap;

struct FakeKeys(HashMap<&'static str, &'static str>);

impl FakeKeys {
    fn new(pairs: &[(&'static str, &'static str)]) -> Self {
        Self(pairs.iter().copied().collect())
    }
}

impl KeySource for FakeKeys {
    fn key(&self, provider: &str) -> Option<String> {
        self.0.get(provider).map(|value| value.to_string())
    }
}

/// A stored sign-in, and whether it can produce a token.
struct Codex(bool, bool);

#[async_trait]
impl CodexSignIn for Codex {
    async fn signed_in(&self) -> bool {
        self.0
    }

    async fn usable(&self) -> bool {
        self.1
    }
}

const SIGNED_IN: Codex = Codex(true, true);
const SIGNED_OUT: Codex = Codex(false, false);
/// Stored, but the refresh is refused.
const UNUSABLE: Codex = Codex(true, false);

fn deepseek_turn() -> ModelSelection {
    ModelSelection {
        provider: "deepseek".into(),
        model: "deepseek-chat".into(),
    }
}

async fn pick(
    explicit: Explicit,
    turn: Option<&ModelSelection>,
    keys: &[(&'static str, &'static str)],
    codex: &Codex,
) -> anyhow::Result<Option<TextModel>> {
    resolve(explicit, turn, &FakeKeys::new(keys), codex).await
}

fn chat(model: &TextModel) -> (&str, &str, Option<Effort>) {
    match &model.transport {
        Transport::Chat {
            base_url,
            api_key,
            reasoning,
        } => (base_url, api_key, *reasoning),
        Transport::Codex { .. } => panic!("expected chat, got {model:?}"),
    }
}

#[tokio::test]
async fn explicit_override_is_used_verbatim() {
    let explicit = Explicit {
        key: Some("sk-explicit".into()),
        base_url: Some("https://example.test/v1".into()),
        model: Some("tiny-writer".into()),
        reasoning: None,
    };
    let resolved = pick(explicit, None, &[], &SIGNED_IN)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(chat(&resolved).0, "https://example.test/v1");
    assert_eq!(resolved.model, "tiny-writer");
    assert_eq!(resolved.source, "explicit");
    assert_eq!(resolved.effort(), "low");
}

#[tokio::test]
async fn a_codex_sign_in_writes_with_gpt_6_sol_at_low_effort_first() {
    let turn = deepseek_turn();
    let resolved = pick(
        Explicit::default(),
        Some(&turn),
        &[("deepseek", "sk-deepseek")],
        &SIGNED_IN,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(resolved.model, "gpt-6-sol");
    assert_eq!(resolved.source, "codex");
    assert_eq!(
        resolved.transport,
        Transport::Codex {
            effort: Effort::Low
        }
    );
    assert_eq!(resolved.effort(), "low");
    assert_eq!(
        resolved.label(),
        "gpt-6-sol effort low (codex, codex responses)"
    );
}

#[tokio::test]
async fn a_usable_default_sign_in_keeps_the_next_source_as_its_fallback() {
    let turn = deepseek_turn();
    let resolved = pick(
        Explicit::default(),
        Some(&turn),
        &[("deepseek", "sk-deepseek")],
        &SIGNED_IN,
    )
    .await
    .unwrap()
    .unwrap();
    let fallback = resolved.fallback.as_deref().unwrap();
    assert_eq!(fallback.model, "deepseek-chat");
    assert_eq!(fallback.source, "turn-model");
    assert_eq!(resolved.note, None);
}

#[tokio::test]
async fn an_unusable_default_sign_in_falls_through_with_a_note() {
    let turn = deepseek_turn();
    let resolved = pick(
        Explicit::default(),
        Some(&turn),
        &[("deepseek", "sk-deepseek")],
        &UNUSABLE,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(resolved.model, "deepseek-chat");
    assert_eq!(resolved.source, "turn-model");
    assert_eq!(resolved.note, Some(CODEX_UNUSABLE));
    assert!(resolved.fallback.is_none());
    // Without a turn model, the provider list stands in the same way.
    let resolved = pick(Explicit::default(), None, &[("xai", "sk-xai")], &UNUSABLE)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resolved.source, "roder-provider");
    assert_eq!(resolved.note, Some(CODEX_UNUSABLE));
    // With nothing to stand in, GPT-6 Sol stays and each fill says why.
    let resolved = pick(Explicit::default(), None, &[], &UNUSABLE)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resolved.model, "gpt-6-sol");
}

#[tokio::test]
async fn an_explicit_choice_never_falls_back() {
    let explicit = Explicit {
        model: Some("gpt-6-sol".into()),
        ..Explicit::default()
    };
    let resolved = pick(explicit, None, &[("deepseek", "sk")], &UNUSABLE)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resolved.model, "gpt-6-sol");
    assert_eq!(resolved.source, "explicit");
    assert!(resolved.fallback.is_none() && resolved.note.is_none());
    // An explicit effort makes the default model explicit too.
    let effort = Explicit {
        reasoning: Some(Effort::Low),
        ..Explicit::default()
    };
    for codex in [&SIGNED_IN, &UNUSABLE] {
        let resolved = pick(effort.clone(), None, &[("deepseek", "sk")], codex)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resolved.model, "gpt-6-sol");
        assert!(resolved.fallback.is_none() && resolved.note.is_none());
    }
}

#[test]
fn the_note_never_carries_credentials_or_auth_errors() {
    for leak in ["token", "refresh", "401", "invalid", "Bearer", "error"] {
        assert!(!CODEX_UNUSABLE.contains(leak), "{leak}");
    }
}

#[tokio::test]
async fn the_reasoning_variable_sets_the_codex_effort() {
    let explicit = Explicit {
        reasoning: Some(Effort::None),
        ..Explicit::default()
    };
    let resolved = pick(explicit, None, &[], &SIGNED_IN)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        resolved.transport,
        Transport::Codex {
            effort: Effort::None
        }
    );
}

#[tokio::test]
async fn an_explicit_model_picks_its_own_provider() {
    // Forced DeepSeek even with a Codex sign-in.
    let explicit = Explicit {
        model: Some("deepseek-chat".into()),
        ..Explicit::default()
    };
    let resolved = pick(explicit, None, &[("deepseek", "sk-deepseek")], &SIGNED_IN)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resolved.model, "deepseek-chat");
    assert_eq!(resolved.source, "explicit");
    assert_eq!(
        chat(&resolved),
        ("https://api.deepseek.com/v1", "sk-deepseek", None)
    );
    assert_eq!(resolved.effort(), "none");

    // An OpenAI model through the sign-in, at the effort asked for.
    let explicit = Explicit {
        model: Some("gpt-6-sol".into()),
        reasoning: Some(Effort::Medium),
        ..Explicit::default()
    };
    let resolved = pick(explicit, None, &[], &SIGNED_IN)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resolved.source, "explicit");
    assert_eq!(
        resolved.transport,
        Transport::Codex {
            effort: Effort::Medium
        }
    );
}

#[tokio::test]
async fn an_explicit_model_nothing_can_serve_is_an_error_not_a_substitute() {
    let unknown = Explicit {
        model: Some("no-such-model".into()),
        ..Explicit::default()
    };
    let error = pick(unknown, None, &[("deepseek", "sk")], &SIGNED_IN)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("no-such-model"), "{error}");

    let keyless = Explicit {
        model: Some("deepseek-chat".into()),
        ..Explicit::default()
    };
    let error = pick(keyless, None, &[], &SIGNED_IN)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("needs a deepseek key"), "{error}");

    // gpt-6-sol with neither a sign-in nor an OpenAI key.
    let signed_out = Explicit {
        model: Some("gpt-6-sol".into()),
        ..Explicit::default()
    };
    assert!(pick(signed_out, None, &[], &SIGNED_OUT).await.is_err());
}

#[test]
fn only_the_four_efforts_parse() {
    assert_eq!(Effort::parse(" LOW ").unwrap(), Effort::Low);
    assert_eq!(Effort::parse("none").unwrap(), Effort::None);
    assert_eq!(Effort::parse("high").unwrap(), Effort::High);
    let error = Effort::parse("max").unwrap_err().to_string();
    assert!(error.contains("none, low, medium or high"), "{error}");
}

#[test]
fn an_effort_the_model_does_not_take_is_refused() {
    // gpt-6-sol's catalog entry lists its efforts; every one we parse is
    // among them.
    for effort in [Effort::None, Effort::Low, Effort::Medium, Effort::High] {
        assert!(codex_model("gpt-6-sol", Some(effort), "codex").is_ok());
    }
    let model = lookup_model("gpt-6-sol").unwrap();
    assert!(
        model
            .supported_reasoning
            .iter()
            .any(|option| option.effort == "low")
    );
}

#[tokio::test]
async fn turn_model_serves_the_text_helper_when_signed_out_and_roder_holds_its_key() {
    let turn = deepseek_turn();
    let resolved = pick(
        Explicit::default(),
        Some(&turn),
        &[("deepseek", "sk-deepseek")],
        &SIGNED_OUT,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(resolved.model, "deepseek-chat");
    assert_eq!(
        chat(&resolved),
        ("https://api.deepseek.com/v1", "sk-deepseek", None)
    );
    assert_eq!(resolved.source, "turn-model");
}

#[tokio::test]
async fn turn_model_on_a_native_transport_does_not_serve_the_text_helper() {
    // Cursor speaks protobuf AgentService, so it cannot answer
    // /chat/completions even though Roder holds its key.
    let selection = ModelSelection {
        provider: "cursor".into(),
        model: "claude-opus-5".into(),
    };
    let resolved = pick(
        Explicit::default(),
        Some(&selection),
        &[("cursor", "sk-cursor"), ("xai", "sk-xai")],
        &SIGNED_OUT,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(resolved.source, "roder-provider");
    assert_eq!(chat(&resolved).1, "sk-xai");
}

#[tokio::test]
async fn oauth_only_turn_model_falls_back_to_a_configured_provider() {
    let selection = ModelSelection {
        provider: "claude-code".into(),
        model: "claude-opus-4-8".into(),
    };
    let resolved = pick(
        Explicit::default(),
        Some(&selection),
        &[("deepseek", "sk-deepseek")],
        &SIGNED_OUT,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(resolved.source, "roder-provider");
    assert_eq!(chat(&resolved).1, "sk-deepseek");
}

#[tokio::test]
async fn fallback_order_prefers_deepseek_over_later_providers() {
    let resolved = pick(
        Explicit::default(),
        None,
        &[("openai", "sk-openai"), ("deepseek", "sk-deepseek")],
        &SIGNED_OUT,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        chat(&resolved),
        ("https://api.deepseek.com/v1", "sk-deepseek", None)
    );
}

#[tokio::test]
async fn openrouter_turns_reasoning_off_unless_told_otherwise() {
    let resolved = pick(
        Explicit::default(),
        None,
        &[("openrouter", "sk-or")],
        &SIGNED_OUT,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(chat(&resolved).2, Some(Effort::None));
    let told = Explicit {
        reasoning: Some(Effort::High),
        ..Explicit::default()
    };
    let resolved = pick(told, None, &[("openrouter", "sk-or")], &SIGNED_OUT)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(chat(&resolved).2, Some(Effort::High));
    assert_eq!(resolved.effort(), "high");
}

#[tokio::test]
async fn no_configured_provider_means_no_text_helper() {
    assert!(
        pick(Explicit::default(), None, &[], &SIGNED_OUT)
            .await
            .unwrap()
            .is_none()
    );
}

#[test]
fn non_chat_completions_providers_are_rejected() {
    assert!(chat_completions_provider("anthropic").is_none());
    assert!(chat_completions_provider("claude-code").is_none());
    assert!(chat_completions_provider("supergrok").is_none());
    assert!(chat_completions_provider("cursor").is_none());
    assert!(chat_completions_provider("deepseek").is_some());
    assert!(chat_completions_provider("openrouter").is_some());
}

#[test]
fn a_label_never_carries_the_key() {
    let model = TextModel {
        model: "deepseek-chat".into(),
        source: "roder-provider",
        note: None,
        fallback: None,
        transport: Transport::Chat {
            base_url: "https://api.deepseek.com/v1".into(),
            api_key: "sk-deepseek-secret".into(),
            reasoning: None,
        },
    };
    let label = model.label();
    assert_eq!(
        label,
        "deepseek-chat effort none (roder-provider, https://api.deepseek.com/v1)"
    );
    assert!(!label.contains("sk-deepseek-secret"));
}
