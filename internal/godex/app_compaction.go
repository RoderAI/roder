package godex

import (
	"context"
	"fmt"
	"strings"

	"github.com/google/uuid"
	"github.com/pandelisz/gode/internal/godex/agent"
	"github.com/pandelisz/gode/internal/godex/eventbus"
	messagestore "github.com/pandelisz/gode/internal/godex/message"
	"github.com/pandelisz/gode/internal/godex/provider"
)

func (a *App) CompactSession(ctx context.Context, sessionID string) (CompactSessionResult, error) {
	sessionID = strings.TrimSpace(sessionID)
	if sessionID == "" {
		return CompactSessionResult{}, fmt.Errorf("session id is required")
	}
	compactor, ok := a.provider.(provider.Compactor)
	if !ok {
		return CompactSessionResult{}, fmt.Errorf("provider %q does not support compaction", a.provider.Name())
	}
	runID := uuid.NewString()
	started := a.publish(ctx, eventbus.Event{
		Kind:      eventbus.KindContextCompactionStarted,
		Source:    eventbus.SourceAgent,
		SessionID: sessionID,
		RunID:     runID,
		Payload:   map[string]any{"model": a.Config.Model},
	})
	a.appendJournal(ctx, started)

	stored, err := a.Messages.ListBySession(ctx, sessionID)
	if err != nil {
		a.recordCompactionFailure(ctx, sessionID, runID, err)
		return CompactSessionResult{}, err
	}
	result, err := compactor.Compact(ctx, provider.CompactRequest{
		SessionID:    sessionID,
		RunID:        runID,
		Model:        a.Config.Model,
		Instructions: agent.GodeInstructions,
		Messages:     providerMessagesFromStored(stored),
	})
	if err != nil {
		a.recordCompactionFailure(ctx, sessionID, runID, err)
		return CompactSessionResult{}, err
	}
	for i, raw := range result.Output {
		text := "compaction item"
		if i == len(result.Output)-1 {
			text = "canonical compacted context"
		}
		if _, err := a.Messages.Append(ctx, messagestore.Message{
			SessionID:  sessionID,
			RunID:      runID,
			Role:       messagestore.RoleCompaction,
			Text:       text,
			RawJSON:    append([]byte(nil), raw...),
			SourceKind: "compacted",
		}); err != nil {
			a.recordCompactionFailure(ctx, sessionID, runID, err)
			return CompactSessionResult{}, err
		}
	}
	if a.Sessions != nil {
		messages, _ := a.Messages.ListBySession(ctx, sessionID)
		_, _ = a.Sessions.UpdateMessageCount(ctx, sessionID, len(messages))
	}
	completed := a.publish(ctx, eventbus.Event{
		Kind:      eventbus.KindContextCompactionCompleted,
		Source:    eventbus.SourceAgent,
		SessionID: sessionID,
		RunID:     runID,
		Payload: map[string]any{
			"model":        a.Config.Model,
			"response_id":  result.ID,
			"output_items": len(result.Output),
		},
	})
	a.appendJournal(ctx, completed)
	return CompactSessionResult{SessionID: sessionID, RunID: runID, ResponseID: result.ID, OutputItems: len(result.Output)}, nil
}

func (a *App) recordCompactionFailure(ctx context.Context, sessionID string, runID string, err error) {
	failed := a.publish(ctx, eventbus.Event{
		Kind:      eventbus.KindContextCompactionFailed,
		Source:    eventbus.SourceAgent,
		SessionID: sessionID,
		RunID:     runID,
		Payload:   map[string]any{"model": a.Config.Model, "error": err.Error()},
	})
	a.appendJournal(ctx, failed)
}

func providerMessagesFromStored(messages []messagestore.Message) []provider.Message {
	messages = canonicalStoredWindow(messages)
	out := make([]provider.Message, 0, len(messages))
	for _, msg := range messages {
		if len(msg.RawJSON) > 0 {
			out = append(out, provider.Message{RawJSON: append([]byte(nil), msg.RawJSON...)})
			continue
		}
		switch msg.Role {
		case messagestore.RoleUser:
			out = append(out, provider.Message{Role: provider.RoleUser, Content: msg.Text})
		case messagestore.RoleAssistant:
			out = append(out, provider.Message{Role: provider.RoleAssistant, Content: msg.Text})
		case messagestore.RoleTool:
			out = append(out, provider.Message{Role: provider.RoleTool, Content: msg.Text, ToolCallID: msg.ToolCallID, ToolName: msg.ToolName})
		}
	}
	return out
}

func canonicalStoredWindow(messages []messagestore.Message) []messagestore.Message {
	latestCompaction := -1
	latestRunID := ""
	for i, msg := range messages {
		if msg.Role == messagestore.RoleCompaction && len(msg.RawJSON) > 0 {
			latestCompaction = i
			latestRunID = msg.RunID
		}
	}
	if latestCompaction == -1 {
		return messages
	}
	start := latestCompaction
	for i := latestCompaction; i >= 0; i-- {
		msg := messages[i]
		if msg.Role != messagestore.RoleCompaction || len(msg.RawJSON) == 0 {
			break
		}
		if latestRunID != "" && msg.RunID != latestRunID {
			break
		}
		start = i
	}
	return messages[start:]
}
