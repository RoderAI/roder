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
