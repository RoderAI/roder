package provider

import "context"

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

type EventKind string

const (
	EventDelta     EventKind = "delta"
	EventToolCall  EventKind = "tool_call"
	EventCompleted EventKind = "completed"
)

type Event struct {
	Kind        EventKind
	Text        string
	ToolRequest *ToolRequest
}

type Request struct {
	SessionID    string
	RunID        string
	Instructions string
	Messages     []Message
	Tools        []ToolSpec
}

type Provider interface {
	Name() string
	Stream(context.Context, Request) (<-chan Event, <-chan error)
}
