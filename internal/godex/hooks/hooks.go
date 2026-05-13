package hooks

import "time"

type Decision string

const (
	DecisionNone  Decision = "none"
	DecisionAllow Decision = "allow"
	DecisionDeny  Decision = "deny"
	DecisionHalt  Decision = "halt"
)

type Hook struct {
	Name    string
	Command string
	Args    []string
	Tools   []string
	Timeout time.Duration
}

type HookInput struct {
	Tool      string
	SessionID string
	Workspace string
	Input     map[string]any
}

type HookResult struct {
	Decision     Decision
	Context      string
	UpdatedInput map[string]any
	Warnings     []string
}
