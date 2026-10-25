use roder_api::inference::TokenUsage;
use roder_api::thread::ThreadUsageMetadata;

fn reported_usage() -> TokenUsage {
    TokenUsage {
        prompt_tokens: 100,
        completion_tokens: 20,
        total_tokens: 120,
        cached_prompt_tokens: 30,
        cache_creation_prompt_tokens: 10,
        ..Default::default()
    }
}

#[test]
fn terminal_usage_deduplication_survives_metadata_reload() {
    let mut metadata = ThreadUsageMetadata::default();
    let turn_id = "turn-1".to_string();
    assert!(metadata.add_turn_token_usage(&turn_id, &reported_usage()));
    let persisted = serde_json::to_vec(&metadata).unwrap();
    let mut restored: ThreadUsageMetadata = serde_json::from_slice(&persisted).unwrap();
    let before_retry = restored.clone();

    assert!(!restored.add_turn_token_usage(&turn_id, &TokenUsage::new(900, 100, 1000)));

    assert_eq!(restored, before_retry);
    assert_eq!(restored.prompt_tokens, 100);
    assert_eq!(restored.completion_tokens, 20);
    assert_eq!(restored.total_tokens, 120);
    assert_eq!(restored.cached_prompt_tokens, 30);
    assert_eq!(restored.cache_creation_prompt_tokens, 10);
    assert_eq!(restored.cache_hit_rate, Some(0.3));
    assert!(restored.accounted_turn_ids.contains(&turn_id));
}

#[test]
fn equal_usage_from_different_turns_is_counted_independently() {
    let mut metadata = ThreadUsageMetadata::default();
    assert!(metadata.add_turn_token_usage(&"turn-1".into(), &reported_usage()));
    assert!(metadata.add_turn_token_usage(&"turn-2".into(), &reported_usage()));

    assert_eq!(metadata.prompt_tokens, 200);
    assert_eq!(metadata.completion_tokens, 40);
    assert_eq!(metadata.total_tokens, 240);
    assert_eq!(metadata.cached_prompt_tokens, 60);
    assert_eq!(metadata.cache_creation_prompt_tokens, 20);
    assert_eq!(metadata.cache_hit_rate, Some(0.3));
    assert_eq!(metadata.accounted_turn_ids.len(), 2);
}

#[test]
fn empty_turn_markers_are_omitted_and_default_when_absent() {
    let serialized = serde_json::to_value(ThreadUsageMetadata::default()).unwrap();
    assert!(serialized.get("accounted_turn_ids").is_none());
    let restored: ThreadUsageMetadata = serde_json::from_value(serialized).unwrap();
    assert!(restored.accounted_turn_ids.is_empty());
}

#[test]
fn non_turn_usage_remains_additive_without_creating_terminal_markers() {
    let mut metadata = ThreadUsageMetadata::default();
    metadata.add_token_usage(&reported_usage());
    metadata.add_token_usage(&reported_usage());

    assert_eq!(metadata.total_tokens, 240);
    assert!(metadata.accounted_turn_ids.is_empty());
    assert!(metadata.add_turn_token_usage(&"turn-1".into(), &reported_usage()));
    assert_eq!(metadata.total_tokens, 360);
    metadata.add_token_usage(&reported_usage());
    assert_eq!(metadata.total_tokens, 480);
    assert_eq!(metadata.accounted_turn_ids.len(), 1);
}
