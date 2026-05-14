package provider

import (
	"encoding/json"
	"errors"
	"testing"
)

func TestRepairAllOrphanFunctionCallOutputsRemovesOnlyUnmatchedOutputs(t *testing.T) {
	messages := []Message{
		{RawJSON: json.RawMessage(`{"type":"function_call_output","call_id":"call_orphan","output":"orphan"}`)},
		{RawJSON: json.RawMessage(`{"type":"function_call","call_id":"call_keep","name":"read_file","arguments":"{}"}`)},
		{RawJSON: json.RawMessage(`{"type":"function_call_output","call_id":"call_keep","output":"kept"}`)},
		{Role: RoleUser, Content: "continue"},
	}

	repaired, callIDs, ok := RepairAllOrphanFunctionCallOutputs(messages)
	if !ok {
		t.Fatal("expected repair")
	}
	if len(callIDs) != 1 || callIDs[0] != "call_orphan" {
		t.Fatalf("call ids = %#v", callIDs)
	}
	if len(repaired) != 3 {
		t.Fatalf("repaired messages = %#v", repaired)
	}
	if !isFunctionCallOutput(repaired[1], "call_keep") {
		t.Fatalf("matched output should remain: %#v", repaired)
	}
}

func TestRepairOrphanFunctionCallOutputUsesProviderErrorCallID(t *testing.T) {
	messages := []Message{
		{RawJSON: json.RawMessage(`{"type":"function_call_output","call_id":"call_missing","output":"orphan"}`)},
		{Role: RoleUser, Content: "continue"},
	}
	err := errors.New(`POST "https://chatgpt.com/backend-api/codex/responses/compact": 400 Bad Request {"message":"No tool call found for function call output with call_id call_missing.","type":"invalid_request_error","param":"input"}`)

	repaired, callID, ok := RepairOrphanFunctionCallOutput(messages, err)
	if !ok || callID != "call_missing" {
		t.Fatalf("repair = %#v callID=%q ok=%t", repaired, callID, ok)
	}
	if len(repaired) != 1 || repaired[0].Content != "continue" {
		t.Fatalf("repaired messages = %#v", repaired)
	}
}

func TestRepairAllOrphanFunctionCallOutputItemsRemovesOnlyUnmatchedOutputs(t *testing.T) {
	items := []Item{
		{Kind: ItemFunctionOut, ToolCallID: "call_orphan", Text: "orphan"},
		{Kind: ItemFunctionCall, ToolCallID: "call_keep", ToolName: "read_file", Text: "{}"},
		{Kind: ItemFunctionOut, ToolCallID: "call_keep", Text: "kept"},
		{Kind: ItemRaw, RawJSON: json.RawMessage(`{"type":"function_call_output","call_id":"call_raw","output":"raw orphan"}`)},
	}
	repaired, callIDs, ok := RepairAllOrphanFunctionCallOutputItems(items)
	if !ok {
		t.Fatal("expected repair")
	}
	if len(callIDs) != 2 || callIDs[0] != "call_orphan" || callIDs[1] != "call_raw" {
		t.Fatalf("call ids = %#v", callIDs)
	}
	if len(repaired) != 2 {
		t.Fatalf("repaired = %#v", repaired)
	}
	if repaired[0].Kind != ItemFunctionCall || repaired[1].Kind != ItemFunctionOut || repaired[1].ToolCallID != "call_keep" {
		t.Fatalf("kept items = %#v", repaired)
	}
}

func TestRepairOrphanFunctionCallOutputItemsUsesProviderErrorCallID(t *testing.T) {
	items := []Item{
		{Kind: ItemFunctionOut, ToolCallID: "call_missing", Text: "orphan"},
		{Kind: ItemMessage, Role: "user", Text: "keep"},
	}
	err := errors.New(`OpenAI stream request failed: 400 Bad Request - No tool call found for function call output with call_id call_missing.`)
	repaired, callID, ok := RepairOrphanFunctionCallOutputItems(items, err)
	if !ok || callID != "call_missing" {
		t.Fatalf("repair = %v callID = %q", ok, callID)
	}
	if len(repaired) != 1 || repaired[0].Text != "keep" {
		t.Fatalf("repaired = %#v", repaired)
	}
}
