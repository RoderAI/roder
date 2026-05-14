package provider

import (
	"context"
	"os"
	"strings"
	"testing"
	"time"
)

func TestChatCompletionsLive(t *testing.T) {
	if strings.TrimSpace(os.Getenv("GODE_LIVE_CHAT_COMPLETIONS")) != "1" {
		t.Skip("set GODE_LIVE_CHAT_COMPLETIONS=1 to run the live Chat Completions smoke test")
	}
	model := strings.TrimSpace(os.Getenv("GODE_MODEL"))
	baseURL := strings.TrimSpace(os.Getenv("GODE_LIVE_CHAT_COMPLETIONS_BASE_URL"))
	apiKey := strings.TrimSpace(os.Getenv("GODE_LIVE_CHAT_COMPLETIONS_API_KEY"))
	if model == "" || baseURL == "" || apiKey == "" {
		t.Skip("GODE_MODEL, GODE_LIVE_CHAT_COMPLETIONS_BASE_URL, and GODE_LIVE_CHAT_COMPLETIONS_API_KEY are required")
	}

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Minute)
	defer cancel()

	chatProvider := NewChatCompletionsWithConfig(ChatCompletionsConfig{
		Model:   model,
		BaseURL: baseURL,
		APIKey:  apiKey,
	})
	events, errs := chatProvider.Stream(ctx, Request{
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
