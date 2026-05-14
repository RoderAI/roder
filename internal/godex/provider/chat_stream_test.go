package provider

import (
	"strings"
	"testing"
)

func TestChatStreamTextOnly(t *testing.T) {
	events := runChatStreamEvents(t,
		`data: {"choices":[{"delta":{"content":"Hello"}}]}`,
		`data: {"choices":[{"delta":{"content":" there"}}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}`,
		`data: {"choices":[{"finish_reason":"stop"}]}`,
		`data: [DONE]`,
	)

	assertProviderEvents(t, events, []EventKind{EventDelta, EventDelta, EventCompleted})
	if events[0].Text != "Hello" || events[1].Text != " there" || events[2].Text != "Hello there" {
		t.Fatalf("events = %#v", events)
	}
	if len(events[2].Items) != 1 || events[2].Items[0].Kind != ItemMessage || events[2].Items[0].Text != "Hello there" {
		t.Fatalf("completed items = %#v", events[2].Items)
	}
	if events[2].Usage.InputTokens != 3 || events[2].Usage.OutputTokens != 2 || events[2].Usage.Total() != 5 {
		t.Fatalf("usage = %#v", events[2].Usage)
	}
}

func TestChatStreamToolCall(t *testing.T) {
	events := runChatStreamEvents(t,
		`data: {"choices":[{"delta":{"content":"I'll inspect that."}}]}`,
		`data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"read_file","arguments":"{\"path\":"}}]}}]}`,
		`data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"README.md\"}"}}]},"finish_reason":"tool_calls"}]}`,
		`data: [DONE]`,
	)

	assertProviderEvents(t, events, []EventKind{EventDelta, EventToolCall, EventCompleted})
	tool := events[1].ToolRequest
	if tool == nil || tool.ID != "call_1" || tool.Name != "read_file" || tool.Arguments != `{"path":"README.md"}` || tool.Input["path"] != "README.md" {
		t.Fatalf("tool request = %#v", tool)
	}
	if len(events[2].Items) != 2 || events[2].Items[0].Kind != ItemMessage || events[2].Items[1].Kind != ItemFunctionCall {
		t.Fatalf("completed items = %#v", events[2].Items)
	}
}

func TestChatStreamMultipleParallelToolCalls(t *testing.T) {
	events := runChatStreamEvents(t,
		`data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"first","arguments":"{}"}},{"index":1,"id":"call_2","type":"function","function":{"name":"second","arguments":"{\"ok\":true}"}}]},"finish_reason":"tool_calls"}]}`,
	)

	assertProviderEvents(t, events, []EventKind{EventToolCall, EventToolCall, EventCompleted})
	if events[0].ToolRequest.Name != "first" || events[1].ToolRequest.Name != "second" {
		t.Fatalf("events = %#v", events)
	}
	if len(events[2].Items) != 2 {
		t.Fatalf("completed items = %#v", events[2].Items)
	}
}

func TestChatStreamMalformedJSONReturnsProviderError(t *testing.T) {
	state := newChatStreamState()
	_, err := state.HandleChatSSELine([]byte(`data: {"choices":[` + strings.Repeat("secret", 80)))
	if err == nil {
		t.Fatal("expected malformed JSON error")
	}
	if !strings.Contains(err.Error(), "malformed chat completion stream chunk") {
		t.Fatalf("error = %v", err)
	}
	if strings.Count(err.Error(), "secret") > 8 {
		t.Fatalf("error should include only a short chunk prefix: %v", err)
	}
}

func TestChatStreamDoneFinishesStream(t *testing.T) {
	state := newChatStreamState()
	events, err := state.HandleChatSSELine([]byte(`data: [DONE]`))
	if err != nil {
		t.Fatalf("done: %v", err)
	}
	if len(events) != 1 || events[0].Kind != EventCompleted || !state.Done() {
		t.Fatalf("events = %#v done = %v", events, state.Done())
	}
}

func TestChatStreamNonStreamingResponse(t *testing.T) {
	state := newChatStreamState()
	events, err := state.HandleChatCompletionResponse([]byte(`{"choices":[{"message":{"role":"assistant","content":"Done","tool_calls":[{"id":"call_1","type":"function","function":{"name":"finish","arguments":"{}"}}]}}],"usage":{"prompt_tokens":4,"completion_tokens":3,"total_tokens":7}}`))
	if err != nil {
		t.Fatalf("handle response: %v", err)
	}
	assertProviderEvents(t, events, []EventKind{EventDelta, EventToolCall, EventCompleted})
	if state.CompletedEvent().Text != "Done" || state.CompletedEvent().Usage.Total() != 7 {
		t.Fatalf("completed = %#v", state.CompletedEvent())
	}
}

func runChatStreamEvents(t *testing.T, rawLines ...string) []Event {
	t.Helper()
	state := newChatStreamState()
	var out []Event
	for _, raw := range rawLines {
		events, err := state.HandleChatSSELine([]byte(raw))
		if err != nil {
			t.Fatalf("handle %s: %v", raw, err)
		}
		out = append(out, events...)
	}
	if len(out) == 0 || out[len(out)-1].Kind != EventCompleted {
		out = append(out, state.CompletedEvent())
	}
	return out
}
