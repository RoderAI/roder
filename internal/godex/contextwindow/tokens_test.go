package contextwindow

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestEstimateMessagesHandlesTextToolsAndRawItems(t *testing.T) {
	window := ForModel("gpt-5.5")
	estimate := EstimateMessages([]Message{
		{Role: "user", Content: "hello"},
		{Role: "assistant", ToolCallID: "call_123", ToolName: "read_file", ToolArguments: `{"path":"README.md"}`},
		{Role: "tool", ToolCallID: "call_123", Content: "path: README.md\nlines: 1-10 of 100"},
		{RawJSON: json.RawMessage(`{"type":"response.compaction","content":"opaque"}`)},
	}, window)
	if estimate.Tokens <= rawItemTokenOverhead {
		t.Fatalf("tokens = %d", estimate.Tokens)
	}
	if estimate.ContextWindow != 1050000 {
		t.Fatalf("context window = %d", estimate.ContextWindow)
	}
	if estimate.Percent <= 0 {
		t.Fatalf("percent = %f", estimate.Percent)
	}
}

func TestEstimateMessagesPercentUsesSelectedWindow(t *testing.T) {
	window := ModelWindow{ContextWindow: 100}
	estimate := EstimateMessages([]Message{{Role: "user", Content: "01234567890123456789"}}, window)
	if estimate.Percent <= 0 || estimate.Percent >= 100 {
		t.Fatalf("percent should use selected window, got %f with %d tokens", estimate.Percent, estimate.Tokens)
	}
}

func TestEstimateRequestCountsInputItemsInstructionsAndTools(t *testing.T) {
	window := ModelWindow{ContextWindow: 100_000}
	estimate := EstimateRequest(Request{
		Model:        "gpt-5.5",
		Instructions: strings.Repeat("system ", 100),
		Messages:     []Message{{Role: "user", Content: "short fallback message"}},
		InputItems: []Item{{
			Kind: "function_call_output",
			Text: strings.Repeat("0123456789abcdef", 2000),
		}},
		Tools: []ToolSpec{{
			Name:        "read_file",
			Description: strings.Repeat("read workspace files ", 50),
			Schema: map[string]any{
				"type":       "object",
				"properties": map[string]any{"path": map[string]any{"type": "string"}},
			},
		}},
	}, window)
	messagesOnly := EstimateMessages([]Message{{Role: "user", Content: "short fallback message"}}, window)
	if estimate.Tokens <= messagesOnly.Tokens {
		t.Fatalf("request estimate did not include input items/instructions/tools: request=%d messages=%d", estimate.Tokens, messagesOnly.Tokens)
	}
}

func TestGPT55FlagsTokenDenseInputAsOverContext(t *testing.T) {
	window := ForModel("gpt-5.5")
	estimate := EstimateMessages([]Message{{Role: "user", Content: strings.Repeat("😀", 1_060_000)}}, window)
	if estimate.Tokens < window.ContextWindow {
		t.Fatalf("tokens = %d", estimate.Tokens)
	}
	if estimate.Percent <= 100 {
		t.Fatalf("estimate should exceed gpt-5.5 context, tokens=%d window=%d percent=%f", estimate.Tokens, window.ContextWindow, estimate.Percent)
	}
}
