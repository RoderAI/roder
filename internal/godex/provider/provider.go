package provider

import (
	"context"
	"encoding/json"
	"fmt"
)

type Role string

const (
	RoleSystem    Role = "system"
	RoleUser      Role = "user"
	RoleAssistant Role = "assistant"
	RoleTool      Role = "tool"
)

const (
	PhaseCommentary  = "commentary"
	PhaseFinalAnswer = "final_answer"
)

type Message struct {
	Role          Role
	Content       string
	Phase         string
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
	Phase      string
	ToolName   string
	ToolCallID string
	Text       string
	RawJSON    json.RawMessage
}

type NonPortableItemError struct {
	ItemID   string
	Kind     string
	Provider string
	Reason   string
}

func (e NonPortableItemError) Error() string {
	return fmt.Sprintf("cannot replay %s item %s with %s: %s", e.Kind, e.ItemID, e.Provider, e.Reason)
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
	Phase       string
	ToolRequest *ToolRequest
	ResponseID  string
	Items       []Item
	Usage       TokenUsage
}

type TokenUsage struct {
	InputTokens  int64
	OutputTokens int64
	TotalTokens  int64
}

func (u TokenUsage) IsZero() bool {
	return u.InputTokens == 0 && u.OutputTokens == 0 && u.Total() == 0
}

func (u TokenUsage) Total() int64 {
	if u.TotalTokens > 0 {
		return u.TotalTokens
	}
	return u.InputTokens + u.OutputTokens
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
