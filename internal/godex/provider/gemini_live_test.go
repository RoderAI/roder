package provider

import (
	"context"
	"os"
	"testing"
	"time"
)

func geminiLiveProvider(t *testing.T) *Gemini {
	return geminiLiveProviderForModel(t, "gemini-3.1-pro-preview")
}

func geminiLiveProviderForModel(t *testing.T, model string) *Gemini {
	t.Helper()
	if os.Getenv("GEMINI_LIVE") != "1" {
		t.Skip("set GEMINI_LIVE=1 to run live Gemini tests")
	}
	key := firstNonEmpty(os.Getenv("GEMINI_API_TOKEN"), os.Getenv("GEMINI_API_KEY"), os.Getenv("GOOGLE_API_KEY"), os.Getenv("GOOGLE_GENAI_API_KEY"), os.Getenv("GOOGLE_AI_API_KEY"))
	if key == "" {
		t.Skip("set GEMINI_API_TOKEN or a supported Gemini API key alias")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	prov, err := NewGeminiWithConfig(ctx, GeminiConfig{Model: model, APIKey: key, Reasoning: "none"})
	if err != nil {
		t.Fatalf("new gemini: %v", err)
	}
	return prov
}

func TestGeminiLiveText(t *testing.T) {
	prov := geminiLiveProvider(t)
	ctx, cancel := context.WithTimeout(context.Background(), 45*time.Second)
	defer cancel()
	events, errs := prov.Stream(ctx, Request{InputItems: []Item{{Kind: ItemMessage, Role: "user", Text: "Reply with exactly: pong"}}})
	got := collectProviderEvents(t, events, errs)
	foundText := false
	for _, ev := range got {
		if ev.Kind == EventCompleted && ev.Text != "" {
			foundText = true
		}
	}
	if !foundText {
		t.Fatalf("no completed text: %#v", got)
	}
}

func TestGeminiLiveToolCall(t *testing.T) {
	prov := geminiLiveProvider(t)
	ctx, cancel := context.WithTimeout(context.Background(), 45*time.Second)
	defer cancel()
	events, errs := prov.Stream(ctx, Request{
		InputItems: []Item{{Kind: ItemMessage, Role: "user", Text: "Call the echo tool with message set to hello."}},
		Tools: []ToolSpec{{Name: "echo", Description: "Echo a message", Schema: map[string]any{
			"type": "object", "properties": map[string]any{"message": map[string]any{"type": "string"}}, "required": []any{"message"},
		}}},
	})
	got := collectProviderEvents(t, events, errs)
	for _, ev := range got {
		if ev.Kind == EventToolCall && ev.ToolRequest != nil && ev.ToolRequest.Name == "echo" {
			return
		}
	}
	t.Fatalf("no tool call: %#v", got)
}

func TestGeminiLiveCustomToolsThoughtSignatureRoundTrip(t *testing.T) {
	prov := geminiLiveProviderForModel(t, "gemini-3.1-pro-preview-customtools")
	tool := ToolSpec{Name: "echo", Description: "Echo a message", Schema: map[string]any{
		"type": "object",
		"properties": map[string]any{
			"message": map[string]any{"type": "string"},
		},
		"required": []any{"message"},
	}}
	firstInput := []Item{{Kind: ItemMessage, Role: "user", Text: "Call the echo tool exactly once with message set to hello. Do not answer in prose before calling the tool."}}
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()
	events, errs := prov.Stream(ctx, Request{InputItems: firstInput, Tools: []ToolSpec{tool}})
	got := collectProviderEvents(t, events, errs)
	var request *ToolRequest
	var completedItems []Item
	for _, ev := range got {
		if ev.Kind == EventToolCall && ev.ToolRequest != nil && ev.ToolRequest.Name == "echo" {
			request = ev.ToolRequest
		}
		if ev.Kind == EventCompleted {
			completedItems = ev.Items
		}
	}
	if request == nil {
		t.Fatalf("no echo tool call: %#v", got)
	}
	var callItem Item
	for _, item := range completedItems {
		if item.Kind == ItemFunctionCall && item.ToolCallID == request.ID {
			callItem = item
			break
		}
	}
	if len(geminiThoughtSignature(callItem.RawJSON)) == 0 {
		t.Fatalf("tool call missing thought signature: %#v", callItem)
	}

	secondInput := append([]Item{}, firstInput...)
	secondInput = append(secondInput, completedItems...)
	secondInput = append(secondInput, Item{Kind: ItemFunctionOut, ToolCallID: request.ID, ToolName: request.Name, Text: `{"message":"hello"}`})
	ctx2, cancel2 := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel2()
	events, errs = prov.Stream(ctx2, Request{InputItems: secondInput, Tools: []ToolSpec{tool}})
	got = collectProviderEvents(t, events, errs)
	for _, ev := range got {
		if ev.Kind == EventCompleted {
			return
		}
	}
	t.Fatalf("round trip did not complete: %#v", got)
}

func TestGeminiLiveCustomToolsSyntheticThoughtSignatureReplay(t *testing.T) {
	prov := geminiLiveProviderForModel(t, "gemini-3.1-pro-preview-customtools")
	tool := ToolSpec{Name: "echo", Description: "Echo a message", Schema: map[string]any{
		"type": "object",
		"properties": map[string]any{
			"message": map[string]any{"type": "string"},
		},
		"required": []any{"message"},
	}}
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()
	events, errs := prov.Stream(ctx, Request{
		InputItems: []Item{
			{Kind: ItemMessage, Role: "user", Text: "Call echo with message hello."},
			{Kind: ItemFunctionCall, ToolCallID: "call_1", ToolName: "echo", Text: `{"message":"hello"}`},
			{Kind: ItemFunctionOut, ToolCallID: "call_1", ToolName: "echo", Text: `{"message":"hello"}`},
			{Kind: ItemMessage, Role: "user", Text: "Reply with exactly: done"},
		},
		Tools: []ToolSpec{tool},
	})
	got := collectProviderEvents(t, events, errs)
	for _, ev := range got {
		if ev.Kind == EventCompleted {
			return
		}
	}
	t.Fatalf("synthetic thought signature replay did not complete: %#v", got)
}

func TestGeminiLiveStructuredOutput(t *testing.T) {
	prov := geminiLiveProvider(t)
	ctx, cancel := context.WithTimeout(context.Background(), 45*time.Second)
	defer cancel()
	events, errs := prov.Stream(ctx, Request{
		InputItems:     []Item{{Kind: ItemMessage, Role: "user", Text: "Return JSON with ok true."}},
		ResponseFormat: `{"type":"json_schema","schema":{"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"]}}`,
	})
	got := collectProviderEvents(t, events, errs)
	for _, ev := range got {
		if ev.Kind == EventCompleted && ev.Text != "" {
			return
		}
	}
	t.Fatalf("no structured output: %#v", got)
}
