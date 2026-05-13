package provider

import (
	"context"
	"encoding/json"
)

type Role string

const (
	RoleSystem    Role = "system"
	RoleUser      Role = "user"
	RoleAssistant Role = "assistant"
	RoleTool      Role = "tool"
)

type Message struct {
	Role          Role
	Content       string
	ToolCallID    string
	ToolName      string
	ToolArguments string
	RawJSON       json.RawMessage
}

type ToolSpec struct {
	Name        string
	Description string
	Schema      map[string]any
}

type ToolRequest struct {
	ID        string
	Name      string
	Input     map[string]any
	Arguments string
}

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
	ID         string
	Kind       ItemKind
	Role       string
	ToolName   string
	ToolCallID string
	Text       string
	RawJSON    json.RawMessage
}

type EventKind string

const (
	EventDelta                 EventKind = "delta"
	EventReasoningSummaryDelta EventKind = "reasoning_summary_delta"
	EventReasoningSummaryDone  EventKind = "reasoning_summary_done"
	EventToolCall              EventKind = "tool_call"
	EventCompleted             EventKind = "completed"
)

type Event struct {
	Kind        EventKind
	Text        string
	ToolRequest *ToolRequest
	ResponseID  string
	Items       []Item
}

type CompactionOptions struct {
	Enabled          bool
	Model            string
	ContextWindow    int
	CompactThreshold int
}

type Request struct {
	SessionID          string
	RunID              string
	Instructions       string
	ResponseFormat     string
	Messages           []Message
	PreviousResponseID string
	InputItems         []Item
	Store              bool
	Tools              []ToolSpec
	Compaction         CompactionOptions
}

type Provider interface {
	Name() string
	Stream(context.Context, Request) (<-chan Event, <-chan error)
}

type CompactRequest struct {
	SessionID    string
	RunID        string
	Model        string
	Instructions string
	Messages     []Message
}

type CompactResult struct {
	ID     string
	Output []json.RawMessage
}

type Compactor interface {
	Compact(context.Context, CompactRequest) (CompactResult, error)
}
