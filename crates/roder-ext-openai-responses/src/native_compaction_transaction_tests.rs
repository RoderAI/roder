use super::*;

fn progress(id: &str) -> InferenceEvent {
    InferenceEvent::Compaction(CompactionProgress {
        status: "completed".into(),
        item_id: Some(id.into()),
        tokens_before: None,
        tokens_after: None,
        duration_ms: None,
        item: Some(json!({"type":"compaction","id":id,"encrypted_content":id})),
    })
}

#[tokio::test]
async fn invalid_compaction_attempts_never_publish_a_replay_boundary() {
    let completion = InferenceEvent::Completed(CompletionMetadata {
        stop_reason: None,
        provider_response_id: None,
    });
    let invalid_endings = vec![
        vec![InferenceEvent::Failed(InferenceFailure {
            message: "failed".into(),
        })],
        vec![progress("second"), completion.clone()],
        vec![
            InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                id: "call".into(),
                name: "write".into(),
                arguments: "{}".into(),
            }),
            completion.clone(),
        ],
        vec![],
    ];
    for ending in invalid_endings {
        let events = vec![
            progress("first"),
            InferenceEvent::Usage(TokenUsage::new(8, 2, 10)),
        ]
        .into_iter()
        .chain(ending)
        .map(Ok);
        let stream: InferenceEventStream = Box::pin(futures::stream::iter(events));
        let mut stream = compaction_stream(stream, vec![]);
        let mut failed = false;
        let mut billed = 0;
        while let Some(event) = stream.next().await {
            match event {
                Err(_) => failed = true,
                Ok(InferenceEvent::Usage(usage)) => billed += usage.total_tokens,
                Ok(InferenceEvent::ProviderMetadata(metadata)) => {
                    assert!(metadata.get("compacted_input").is_none());
                }
                Ok(InferenceEvent::Completed(_)) => panic!("invalid attempt completed"),
                _ => {}
            }
        }
        assert!(failed);
        assert_eq!(billed, 10);
    }
    let stream: InferenceEventStream = Box::pin(futures::stream::iter(vec![Ok(completion)]));
    assert!(
        compaction_stream(stream, vec![])
            .next()
            .await
            .unwrap()
            .is_err()
    );
}

#[tokio::test]
async fn malformed_opaque_state_cannot_be_committed_by_terminal_completion() {
    let mut malformed = progress("empty");
    if let InferenceEvent::Compaction(progress) = &mut malformed {
        progress.item.as_mut().unwrap()["encrypted_content"] = json!("");
    }
    let stream: InferenceEventStream = Box::pin(futures::stream::iter(vec![
        Ok(malformed),
        Ok(InferenceEvent::Completed(CompletionMetadata {
            stop_reason: None,
            provider_response_id: None,
        })),
    ]));
    assert!(
        compaction_stream(stream, vec![])
            .next()
            .await
            .unwrap()
            .is_err()
    );
}
