package message

import (
	"encoding/json"
	"time"
)

const (
	RoleUser       = "user"
	RoleAssistant  = "assistant"
	RoleTool       = "tool"
	RoleError      = "error"
	RoleCompaction = "compaction"
)

type Message struct {
	ID         string          `json:"id"`
	SessionID  string          `json:"session_id"`
	RunID      string          `json:"run_id,omitempty"`
	Role       string          `json:"role"`
	Text       string          `json:"text"`
	ToolName   string          `json:"tool_name,omitempty"`
	ToolCallID string          `json:"tool_call_id,omitempty"`
	RawJSON    json.RawMessage `json:"raw_json,omitempty"`
	SourceKind string          `json:"source_kind,omitempty"`
	CreatedAt  time.Time       `json:"created_at"`
}
