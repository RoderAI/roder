package provider

import (
	"strings"
	"testing"
)

func TestLocalPrunedMessagesDropsOldestContextAndAddsMarker(t *testing.T) {
	pruned, dropped, ok := LocalPrunedMessages(LocalPruneRequest{
		Model: "gpt-5.5",
		Messages: []Message{
			{Role: RoleUser, Content: strings.Repeat("old context ", 2000)},
			{Role: RoleAssistant, Content: strings.Repeat("old answer ", 2000)},
			{Role: RoleUser, Content: "current prompt"},
		},
		TargetTokens: 100,
	})
	if !ok {
		t.Fatal("expected local prune")
	}
	if dropped == 0 {
		t.Fatalf("dropped = %d", dropped)
	}
	if len(pruned) != 2 {
		t.Fatalf("pruned messages = %#v", pruned)
	}
	if !strings.Contains(string(pruned[0].RawJSON), LocalPruneMarkerText) {
		t.Fatalf("missing prune marker: %#v", pruned[0])
	}
	if pruned[1].Content != "current prompt" {
		t.Fatalf("latest prompt should be retained: %#v", pruned[1])
	}
}

func TestShouldPruneAfterCompactionErrorDetectsContentTypeDecodeFailure(t *testing.T) {
	err := &ProviderError{Message: "expected destination type of 'string' or '[]byte' for responses with content-type '' that is not 'application/json'"}
	if !ShouldPruneAfterCompactionError(err) {
		t.Fatal("expected content-type decode failure to trigger local prune")
	}
}
