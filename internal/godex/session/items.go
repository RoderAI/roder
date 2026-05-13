package session

import (
	"encoding/json"
	"time"
)

type ItemKind string

const (
	ItemMessage      ItemKind = "message"
	ItemFunctionCall ItemKind = "function_call"
	ItemFunctionOut  ItemKind = "function_call_output"
	ItemReasoning    ItemKind = "reasoning"
	ItemCompaction   ItemKind = "compaction"
	ItemRaw          ItemKind = "raw"
)

type Item struct {
	ID         string          `json:"id"`
	SessionID  string          `json:"session_id"`
	TurnID     string          `json:"turn_id"`
	Kind       ItemKind        `json:"kind"`
	Role       string          `json:"role,omitempty"`
	Phase      string          `json:"phase,omitempty"`
	ToolName   string          `json:"tool_name,omitempty"`
	ToolCallID string          `json:"tool_call_id,omitempty"`
	Text       string          `json:"text,omitempty"`
	Images     []Image         `json:"images,omitempty"`
	RawJSON    json.RawMessage `json:"raw_json,omitempty"`
	CreatedAt  time.Time       `json:"created_at"`
}

type Image struct {
	URL    string `json:"url"`
	Detail string `json:"detail,omitempty"`
}
