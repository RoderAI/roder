package provider

import (
	"encoding/json"
	"strings"
	"testing"

	anthropic "github.com/anthropics/anthropic-sdk-go"
)

func TestAnthropicStreamTextOnly(t *testing.T) {
	events := runAnthropicStreamEvents(t,
		`{"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","content":[],"model":"claude-sonnet-4-6","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":1}}}`,
		`{"type":"content_block_start","index":0,"content_block":{"type":"text","text":"Hello"}}`,
		`{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" there"}}`,
		`{"type":"content_block_stop","index":0}`,
		`{"type":"message_stop"}`,
	)

	assertProviderEvents(t, events, []EventKind{EventDelta, EventDelta, EventCompleted})
	if events[0].Text != "Hello" || events[1].Text != " there" || events[2].Text != "Hello there" {
		t.Fatalf("events = %#v", events)
	}
	if len(events[2].Items) != 1 || events[2].Items[0].Kind != ItemMessage || events[2].Items[0].Text != "Hello there" {
		t.Fatalf("completed items = %#v", events[2].Items)
	}
	if events[2].Usage.InputTokens != 1 || events[2].Usage.OutputTokens != 1 || events[2].Usage.Total() != 2 {
		t.Fatalf("usage = %#v", events[2].Usage)
	}
}

func TestAnthropicStreamTextThenTool(t *testing.T) {
	events := runAnthropicStreamEvents(t,
		`{"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","content":[],"model":"claude-sonnet-4-6","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":1}}}`,
		`{"type":"content_block_start","index":0,"content_block":{"type":"text","text":"I'll inspect that."}}`,
		`{"type":"content_block_stop","index":0}`,
		`{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01","name":"read_file","input":{}}}`,
		`{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":"}}`,
		`{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"README.md\"}"}}`,
		`{"type":"content_block_stop","index":1}`,
		`{"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":12}}`,
		`{"type":"message_stop"}`,
	)

	assertProviderEvents(t, events, []EventKind{EventDelta, EventToolCall, EventCompleted})
	tool := events[1].ToolRequest
	if tool == nil || tool.ID != "toolu_01" || tool.Name != "read_file" || tool.Arguments != `{"path":"README.md"}` || tool.Input["path"] != "README.md" {
		t.Fatalf("tool request = %#v", tool)
	}
	if events[2].Text != "I'll inspect that." {
		t.Fatalf("completed text = %q", events[2].Text)
	}
	if len(events[2].Items) != 2 || events[2].Items[0].Kind != ItemMessage || events[2].Items[1].Kind != ItemFunctionCall {
		t.Fatalf("completed items = %#v", events[2].Items)
	}
	if events[2].Usage.InputTokens != 1 || events[2].Usage.OutputTokens != 12 || events[2].Usage.Total() != 13 {
		t.Fatalf("usage = %#v", events[2].Usage)
	}
}

func TestAnthropicStreamMultipleToolUses(t *testing.T) {
	events := runAnthropicStreamEvents(t,
		`{"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","content":[],"model":"claude-sonnet-4-6","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":1}}}`,
		`{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_01","name":"first","input":{}}}`,
		`{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{}"}}`,
		`{"type":"content_block_stop","index":0}`,
		`{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_02","name":"second","input":{}}}`,
		`{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"ok\":true}"}}`,
		`{"type":"content_block_stop","index":1}`,
		`{"type":"message_stop"}`,
	)

	assertProviderEvents(t, events, []EventKind{EventToolCall, EventToolCall, EventCompleted})
	if events[0].ToolRequest.Name != "first" || events[1].ToolRequest.Name != "second" {
		t.Fatalf("events = %#v", events)
	}
	if len(events[2].Items) != 2 {
		t.Fatalf("completed items = %#v", events[2].Items)
	}
}

func TestAnthropicStreamMalformedToolJSONStillEmitsRawArguments(t *testing.T) {
	events := runAnthropicStreamEvents(t,
		`{"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","content":[],"model":"claude-sonnet-4-6","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":1}}}`,
		`{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_bad","name":"read_file","input":{}}}`,
		`{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":"}}`,
		`{"type":"content_block_stop","index":0}`,
		`{"type":"message_stop"}`,
	)

	assertProviderEvents(t, events, []EventKind{EventToolCall, EventCompleted})
	if events[0].ToolRequest.Arguments != `{"path":` {
		t.Fatalf("arguments = %q", events[0].ToolRequest.Arguments)
	}
	if len(events[0].ToolRequest.Input) != 0 {
		t.Fatalf("malformed input should decode to empty map: %#v", events[0].ToolRequest.Input)
	}
}

func TestAnthropicStreamRejectsInvalidEventOrder(t *testing.T) {
	state := newAnthropicStreamState()
	_, err := state.Handle(anthropicEvent(t, `{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"oops"}}`))
	if err == nil || !strings.Contains(err.Error(), "there was no content block") {
		t.Fatalf("error = %v", err)
	}
}

func runAnthropicStreamEvents(t *testing.T, rawEvents ...string) []Event {
	t.Helper()
	state := newAnthropicStreamState()
	var out []Event
	for _, raw := range rawEvents {
		events, err := state.Handle(anthropicEvent(t, raw))
		if err != nil {
			t.Fatalf("handle %s: %v", raw, err)
		}
		out = append(out, events...)
	}
	out = append(out, state.CompletedEvent())
	return out
}

func anthropicEvent(t *testing.T, raw string) anthropic.MessageStreamEventUnion {
	t.Helper()
	var event anthropic.MessageStreamEventUnion
	if err := json.Unmarshal([]byte(raw), &event); err != nil {
		t.Fatalf("unmarshal event: %v\n%s", err, raw)
	}
	return event
}

func assertProviderEvents(t *testing.T, events []Event, want []EventKind) {
	t.Helper()
	if len(events) != len(want) {
		t.Fatalf("events = %#v, want kinds %#v", events, want)
	}
	for i := range want {
		if events[i].Kind != want[i] {
			t.Fatalf("event %d kind = %s, want %s; events = %#v", i, events[i].Kind, want[i], events)
		}
	}
}
