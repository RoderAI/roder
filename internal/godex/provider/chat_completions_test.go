package provider

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

func TestChatCompletionsRequestURLAuthAndPayload(t *testing.T) {
	var gotPath string
	var gotAuth string
	var gotRequest chatCompletionRequest
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotPath = r.URL.Path
		gotAuth = r.Header.Get("Authorization")
		if err := json.NewDecoder(r.Body).Decode(&gotRequest); err != nil {
			t.Fatalf("decode request: %v", err)
		}
		w.Header().Set("Content-Type", "text/event-stream")
		fmt.Fprintln(w, `data: {"choices":[{"delta":{"content":"hello"}}]}`)
		fmt.Fprintln(w, `data: [DONE]`)
	}))
	defer server.Close()

	prov := NewChatCompletionsWithConfig(ChatCompletionsConfig{
		Model:   "deepseek-chat",
		BaseURL: server.URL + "/v1/",
		APIKey:  "secret-key",
	})
	events, errs := prov.Stream(context.Background(), Request{
		PreviousResponseID: "resp_should_not_be_sent",
		InputItems:         []Item{{Kind: ItemMessage, Role: "user", Text: "hi"}},
		Tools:              []ToolSpec{{Name: "read_file", Schema: map[string]any{"type": "object"}}},
	})
	gotEvents := collectProviderEvents(t, events, errs)
	if gotPath != "/v1/chat/completions" {
		t.Fatalf("path = %q", gotPath)
	}
	if gotAuth != "Bearer secret-key" {
		t.Fatalf("authorization = %q", gotAuth)
	}
	if gotRequest.Model != "deepseek-chat" || !gotRequest.Stream || len(gotRequest.Messages) != 1 || len(gotRequest.Tools) != 1 {
		t.Fatalf("request = %#v", gotRequest)
	}
	data, err := json.Marshal(gotRequest)
	if err != nil {
		t.Fatalf("marshal request: %v", err)
	}
	if strings.Contains(string(data), "resp_should_not_be_sent") {
		t.Fatalf("previous response id leaked into request: %s", data)
	}
	if len(gotEvents) != 2 || gotEvents[0].Kind != EventDelta || gotEvents[1].Kind != EventCompleted {
		t.Fatalf("events = %#v", gotEvents)
	}
}

func TestChatCompletionsCustomHeaders(t *testing.T) {
	var gotStatic string
	var gotEnv string
	t.Setenv("CHAT_COMPLETIONS_ENV_HEADER", "env-value")
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotStatic = r.Header.Get("X-Static")
		gotEnv = r.Header.Get("X-Env")
		w.Header().Set("Content-Type", "text/event-stream")
		fmt.Fprintln(w, `data: [DONE]`)
	}))
	defer server.Close()

	prov := NewChatCompletionsWithConfig(ChatCompletionsConfig{
		Model:   "kimi-k2.6",
		BaseURL: server.URL,
		Headers: map[string]string{
			"X-Static": "static-value",
		},
		HeaderEnv: map[string]string{
			"X-Env": "CHAT_COMPLETIONS_ENV_HEADER",
		},
	})
	events, errs := prov.Stream(context.Background(), Request{InputItems: []Item{{Kind: ItemMessage, Role: "user", Text: "hi"}}})
	collectProviderEvents(t, events, errs)
	if gotStatic != "static-value" || gotEnv != "env-value" {
		t.Fatalf("headers static=%q env=%q", gotStatic, gotEnv)
	}
}

func TestChatCompletionsHTTPErrorIncludesSafeDetails(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Error(w, `{"error":"bad key"}`, http.StatusUnauthorized)
	}))
	defer server.Close()

	prov := NewChatCompletionsWithConfig(ChatCompletionsConfig{Model: "deepseek-chat", BaseURL: server.URL, APIKey: "secret-key"})
	_, errs := prov.Stream(context.Background(), Request{InputItems: []Item{{Kind: ItemMessage, Role: "user", Text: "hi"}}})
	err := <-errs
	if err == nil {
		t.Fatal("expected error")
	}
	if !strings.Contains(err.Error(), "401 Unauthorized") || !strings.Contains(err.Error(), "bad key") {
		t.Fatalf("error = %v", err)
	}
	if strings.Contains(err.Error(), "secret-key") {
		t.Fatalf("error leaked secret: %v", err)
	}
}

func TestChatCompletionsContextCancellationClosesResponse(t *testing.T) {
	requestDone := make(chan struct{})
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		fmt.Fprintln(w, `data: {"choices":[{"delta":{"content":"hello"}}]}`)
		if flusher, ok := w.(http.Flusher); ok {
			flusher.Flush()
		}
		<-r.Context().Done()
		close(requestDone)
	}))
	defer server.Close()

	prov := NewChatCompletionsWithConfig(ChatCompletionsConfig{Model: "deepseek-chat", BaseURL: server.URL})
	ctx, cancel := context.WithCancel(context.Background())
	events, errs := prov.Stream(ctx, Request{InputItems: []Item{{Kind: ItemMessage, Role: "user", Text: "hi"}}})
	if ev := <-events; ev.Kind != EventDelta || ev.Text != "hello" {
		t.Fatalf("first event = %#v", ev)
	}
	cancel()
	select {
	case <-requestDone:
	case <-time.After(2 * time.Second):
		t.Fatal("server did not observe cancelled request")
	}
	for range events {
	}
	if err := <-errs; err == nil {
		t.Fatal("expected cancellation error")
	}
}

func collectProviderEvents(t *testing.T, events <-chan Event, errs <-chan error) []Event {
	t.Helper()
	var out []Event
	for ev := range events {
		out = append(out, ev)
	}
	if err := <-errs; err != nil {
		t.Fatalf("stream error: %v", err)
	}
	return out
}
