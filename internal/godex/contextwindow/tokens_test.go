package contextwindow

import (
	"encoding/json"
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
