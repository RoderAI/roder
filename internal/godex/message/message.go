package message

import "time"

const (
	RoleUser      = "user"
	RoleAssistant = "assistant"
	RoleTool      = "tool"
	RoleError     = "error"
)

type Message struct {
	ID         string    `json:"id"`
	SessionID  string    `json:"session_id"`
	RunID      string    `json:"run_id,omitempty"`
	Role       string    `json:"role"`
	Text       string    `json:"text"`
	ToolName   string    `json:"tool_name,omitempty"`
	ToolCallID string    `json:"tool_call_id,omitempty"`
	SourceKind string    `json:"source_kind,omitempty"`
	CreatedAt  time.Time `json:"created_at"`
}
