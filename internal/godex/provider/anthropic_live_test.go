//go:build e2e

package provider

import (
	"context"
	"os"
	"strings"
	"testing"
	"time"
)

func TestAnthropicLiveSmoke(t *testing.T) {
	apiKey := strings.TrimSpace(os.Getenv("ANTHROPIC_API_KEY"))
	if apiKey == "" {
		t.Skip("ANTHROPIC_API_KEY is required for Anthropic live smoke")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Minute)
	defer cancel()

	anthropicProvider := NewAnthropicWithConfig(AnthropicConfig{
		Model:     "claude-sonnet-4-6",
		MaxTokens: 64,
		APIKey:    apiKey,
	})
	events, errs := anthropicProvider.Stream(ctx, Request{
		InputItems: []Item{{Kind: ItemMessage, Role: "user", Text: "Reply with the word gode."}},
	})
	var final string
	for events != nil || errs != nil {
		select {
		case ev, ok := <-events:
			if !ok {
				events = nil
				continue
			}
			if ev.Kind == EventCompleted {
				final = ev.Text
			}
		case err, ok := <-errs:
			if !ok {
				errs = nil
				continue
			}
			if err != nil {
				t.Fatalf("stream: %v", err)
			}
		case <-ctx.Done():
			t.Fatal(ctx.Err())
		}
	}
	if !strings.Contains(strings.ToLower(final), "gode") {
		t.Fatalf("final = %q", final)
	}
}
