package provider

import (
	"context"
	"errors"
	"strings"
	"testing"
	"time"

	"google.golang.org/genai"
)

func TestGeminiStreamTextToolCallUsageAndCompleted(t *testing.T) {
	state := newGeminiStreamState()
	events, err := state.Handle(&genai.GenerateContentResponse{
		ResponseID: "resp_1",
		Candidates: []*genai.Candidate{{Content: &genai.Content{Parts: []*genai.Part{
			{Text: "hello "},
			{FunctionCall: &genai.FunctionCall{ID: "call_1", Name: "read_file", Args: map[string]any{"path": "README.md"}}, ThoughtSignature: []byte("sig")},
		}}}},
		UsageMetadata: &genai.GenerateContentResponseUsageMetadata{PromptTokenCount: 11, CandidatesTokenCount: 7, TotalTokenCount: 18},
	})
	if err != nil {
		t.Fatalf("handle: %v", err)
	}
	if len(events) != 2 || events[0].Kind != EventDelta || events[1].Kind != EventToolCall {
		t.Fatalf("events = %#v", events)
	}
	tool := events[1].ToolRequest
	if tool.ID != "call_1" || tool.Name != "read_file" || tool.Input["path"] != "README.md" || !strings.Contains(tool.Arguments, "README.md") {
		t.Fatalf("tool = %#v", tool)
	}
	if got := string(geminiThoughtSignature(events[1].Items[0].RawJSON)); got != "sig" {
		t.Fatalf("tool event thought signature = %q raw=%s", got, events[1].Items[0].RawJSON)
	}
	completed := state.CompletedEvent()
	if completed.ResponseID != "resp_1" || completed.Text != "hello " || completed.Usage.Total() != 18 {
		t.Fatalf("completed = %#v", completed)
	}
	if len(completed.Items) != 2 || completed.Items[1].Kind != ItemFunctionCall || completed.Items[1].ToolCallID != "call_1" {
		t.Fatalf("items = %#v", completed.Items)
	}
	if got := string(geminiThoughtSignature(completed.Items[1].RawJSON)); got != "sig" {
		t.Fatalf("completed thought signature = %q raw=%s", got, completed.Items[1].RawJSON)
	}
}

func TestGeminiStreamErrorsOnBlockedPrompt(t *testing.T) {
	_, err := newGeminiStreamState().Handle(&genai.GenerateContentResponse{PromptFeedback: &genai.GenerateContentResponsePromptFeedback{BlockReason: genai.BlockedReasonSafety}})
	if err == nil || !strings.Contains(err.Error(), "blocked") {
		t.Fatalf("error = %v", err)
	}
}

func TestGeminiProviderStreamCancellation(t *testing.T) {
	prov := newGeminiWithClient(GeminiConfig{Model: "gemini-3.1-pro-preview"}, fakeGeminiClient{responses: []*genai.GenerateContentResponse{{Candidates: []*genai.Candidate{{Content: &genai.Content{Parts: []*genai.Part{{Text: "hello"}}}}}}}}, genai.BackendGeminiAPI, "", "")
	ctx, cancel := context.WithCancel(context.Background())
	events, errs := prov.Stream(ctx, Request{InputItems: []Item{{Kind: ItemMessage, Role: "user", Text: "hi"}}})
	if ev := <-events; ev.Kind != EventDelta || ev.Text != "hello" {
		t.Fatalf("event = %#v", ev)
	}
	cancel()
	for range events {
	}
	select {
	case err := <-errs:
		if err == nil {
			t.Fatal("expected cancellation error")
		}
	case <-time.After(2 * time.Second):
		t.Fatal("timed out waiting for error")
	}
}

func TestGeminiProviderStreamSDKError(t *testing.T) {
	prov := newGeminiWithClient(GeminiConfig{Model: "gemini-3.1-pro-preview"}, fakeGeminiClient{err: errors.New("boom")}, genai.BackendGeminiAPI, "", "")
	_, errs := prov.Stream(context.Background(), Request{InputItems: []Item{{Kind: ItemMessage, Role: "user", Text: "hi"}}})
	if err := <-errs; err == nil || !strings.Contains(err.Error(), "Gemini stream failed") {
		t.Fatalf("error = %v", err)
	}
}
