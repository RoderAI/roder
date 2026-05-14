package eventbus

import (
	"encoding/json"
	"time"
)

type Source string

const (
	SourceAgent    Source = "agent"
	SourceProvider Source = "provider"
	SourceTool     Source = "tool"
	SourceTUI      Source = "tui"
	SourceMCP      Source = "mcp"
	SourceLSP      Source = "lsp"
	SourcePlugin   Source = "plugin"
	SourceSystem   Source = "system"
)

type Kind string

const (
	KindUserPromptSubmitted Kind = "user.prompt_submitted"
	KindUserSteerSubmitted  Kind = "user.steer_submitted"
	KindUserSteerApplied    Kind = "user.steer_applied"
	KindRunStarted          Kind = "run.started"
	KindRunCompleted        Kind = "run.completed"
	KindRunFailed           Kind = "run.failed"
	KindRunCancelRequested  Kind = "run.cancel_requested"

	KindProviderDelta Kind = "provider.delta"

	KindContextTokensUpdated        Kind = "context.tokens_updated"
	KindContextCompactionConfigured Kind = "context.compaction_configured"
	KindContextCompactionStarted    Kind = "context.compaction_started"
	KindContextCompactionRepaired   Kind = "context.compaction_repaired"
	KindContextCompactionCompleted  Kind = "context.compaction_completed"
	KindContextCompactionFailed     Kind = "context.compaction_failed"

	KindGoalUpdated Kind = "goal.updated"
	KindGoalCleared Kind = "goal.cleared"
	KindGoalLimited Kind = "goal.budget_limited"

	KindMemoryRecalled          Kind = "memory.recalled"
	KindMemoryRecallFailed      Kind = "memory.recall_failed"
	KindMemoryObserverStarted   Kind = "memory.observer_started"
	KindMemoryObserverCompleted Kind = "memory.observer_completed"
	KindMemoryObserverFailed    Kind = "memory.observer_failed"

	KindAssistantDelta            Kind = "assistant.delta"
	KindAssistantCompleted        Kind = "assistant.completed"
	KindReasoningSummaryDelta     Kind = "reasoning_summary.delta"
	KindReasoningSummaryCompleted Kind = "reasoning_summary.completed"

	KindToolRequested Kind = "tool.requested"
	KindToolStarted   Kind = "tool.started"
	KindToolCompleted Kind = "tool.completed"
	KindToolFailed    Kind = "tool.failed"

	KindPermissionRequested Kind = "permission.requested"
	KindPermissionResponded Kind = "permission.responded"
	KindSettingsChanged     Kind = "settings.changed"

	KindMCPStateChanged Kind = "mcp.state_changed"
	KindLSPStateChanged Kind = "lsp.state_changed"
	KindLSPDiagnostics  Kind = "lsp.diagnostics"
	KindPluginLog       Kind = "plugin.log"
	KindSubscriberDrop  Kind = "subscriber.dropped"

	KindSessionProjected Kind = "session.projected"
)

type Event struct {
	ID            string    `json:"id"`
	Seq           uint64    `json:"seq"`
	Time          time.Time `json:"time"`
	SessionID     string    `json:"session_id,omitempty"`
	RunID         string    `json:"run_id,omitempty"`
	Source        Source    `json:"source"`
	Kind          Kind      `json:"kind"`
	CorrelationID string    `json:"correlation_id,omitempty"`
	Payload       any       `json:"payload,omitempty"`
}

func NewEvent(kind Kind, source Source, payload any) Event {
	ev := Event{Kind: kind, Source: source}
	ev.SetPayload(payload)
	return ev
}

func (e *Event) SetPayload(payload any) {
	if payload == nil {
		e.Payload = nil
		return
	}
	e.Payload = payload
}

func (e Event) DecodePayload(dst any) error {
	if e.Payload == nil {
		return nil
	}
	if raw, ok := e.Payload.(json.RawMessage); ok {
		return json.Unmarshal(raw, dst)
	}
	data, err := json.Marshal(e.Payload)
	if err != nil {
		return err
	}
	return json.Unmarshal(data, dst)
}

type Filter struct {
	SessionID     string
	RunID         string
	CorrelationID string
	Kinds         []Kind
	Sources       []Source
}

func (f Filter) Match(e Event) bool {
	if f.SessionID != "" && f.SessionID != e.SessionID {
		return false
	}
	if f.RunID != "" && f.RunID != e.RunID {
		return false
	}
	if f.CorrelationID != "" && f.CorrelationID != e.CorrelationID {
		return false
	}
	if len(f.Kinds) > 0 {
		found := false
		for _, kind := range f.Kinds {
			if e.Kind == kind {
				found = true
				break
			}
		}
		if !found {
			return false
		}
	}
	if len(f.Sources) > 0 {
		found := false
		for _, source := range f.Sources {
			if e.Source == source {
				found = true
				break
			}
		}
		if !found {
			return false
		}
	}
	return true
}
