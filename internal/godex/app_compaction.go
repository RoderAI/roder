package godex

import (
	"context"
	"fmt"
	"strings"

	"github.com/google/uuid"
	"github.com/pandelisz/gode/internal/godex/agent"
	"github.com/pandelisz/gode/internal/godex/contextwindow"
	"github.com/pandelisz/gode/internal/godex/eventbus"
	messagestore "github.com/pandelisz/gode/internal/godex/message"
	"github.com/pandelisz/gode/internal/godex/provider"
	"github.com/pandelisz/gode/internal/godex/session"
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

	messages, err := a.compactionSourceMessages(ctx, sessionID)
	if err != nil {
		a.recordCompactionFailure(ctx, sessionID, runID, err)
		return CompactSessionResult{}, err
	}
	result, err := a.compactWithOrphanRepair(ctx, compactor, sessionID, runID, messages)
	if err != nil {
		if provider.ShouldPruneAfterCompactionError(err) {
			if result, pruneErr := a.persistLocalPrunedCompaction(ctx, sessionID, runID, messages, err); pruneErr == nil {
				a.publishCompactionCompleted(ctx, sessionID, runID, result.ResponseID, result.OutputItems)
				return result, nil
			}
		}
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
	if a.Items != nil {
		items := make([]session.Item, 0, len(result.Output))
		for i, raw := range result.Output {
			items = append(items, session.Item{
				ID:        fmt.Sprintf("%s:manual-compaction:%d", runID, i),
				SessionID: sessionID,
				TurnID:    runID,
				Kind:      session.ItemCompaction,
				RawJSON:   append([]byte(nil), raw...),
			})
		}
		if _, err := a.Items.AppendMany(ctx, items); err != nil {
			a.recordCompactionFailure(ctx, sessionID, runID, err)
			return CompactSessionResult{}, err
		}
	}
	if a.Sessions != nil {
		messages, _ := a.Messages.ListBySession(ctx, sessionID)
		_, _ = a.Sessions.UpdateMessageCount(ctx, sessionID, len(messages))
	}
	a.publishCompactionCompleted(ctx, sessionID, runID, result.ID, len(result.Output))
	return CompactSessionResult{SessionID: sessionID, RunID: runID, ResponseID: result.ID, OutputItems: len(result.Output)}, nil
}

func (a *App) publishCompactionCompleted(ctx context.Context, sessionID string, runID string, responseID string, outputItems int) {
	completed := a.publish(ctx, eventbus.Event{
		Kind:      eventbus.KindContextCompactionCompleted,
		Source:    eventbus.SourceAgent,
		SessionID: sessionID,
		RunID:     runID,
		Payload: map[string]any{
			"model":        a.Config.Model,
			"response_id":  responseID,
			"output_items": outputItems,
		},
	})
	a.appendJournal(ctx, completed)
}

func (a *App) compactionSourceMessages(ctx context.Context, sessionID string) ([]provider.Message, error) {
	if a.Items != nil {
		items, err := a.Items.ListBySession(ctx, sessionID)
		if err != nil {
			return nil, err
		}
		if messages := providerMessagesFromSessionItems(items); len(messages) > 0 {
			return messages, nil
		}
	}
	stored, err := a.Messages.ListBySession(ctx, sessionID)
	if err != nil {
		return nil, err
	}
	return providerMessagesFromStored(stored), nil
}

func (a *App) persistLocalPrunedCompaction(ctx context.Context, sessionID string, runID string, messages []provider.Message, originalErr error) (CompactSessionResult, error) {
	options := contextwindow.OptionsForModel(a.Config.Model, a.Config.DisableAutoCompaction, a.Config.AutoCompactTokenLimit)
	pruned, dropped, ok := provider.LocalPrunedMessages(provider.LocalPruneRequest{
		Model:        a.Config.Model,
		Instructions: agent.GodeInstructions,
		Messages:     messages,
		TargetTokens: options.CompactThreshold,
	})
	if !ok {
		return CompactSessionResult{}, originalErr
	}
	a.recordLocalPruneRepair(ctx, sessionID, runID, dropped, originalErr)
	if err := a.appendPrunedMessages(ctx, sessionID, runID, pruned); err != nil {
		return CompactSessionResult{}, err
	}
	if err := a.appendPrunedItems(ctx, sessionID, runID, pruned); err != nil {
		return CompactSessionResult{}, err
	}
	if a.Sessions != nil {
		messages, _ := a.Messages.ListBySession(ctx, sessionID)
		_, _ = a.Sessions.UpdateMessageCount(ctx, sessionID, len(messages))
	}
	return CompactSessionResult{SessionID: sessionID, RunID: runID, ResponseID: "local_prune", OutputItems: len(pruned)}, nil
}

func (a *App) appendPrunedMessages(ctx context.Context, sessionID string, runID string, messages []provider.Message) error {
	if len(messages) == 0 {
		return nil
	}
	if _, err := a.Messages.Append(ctx, messagestore.Message{
		SessionID:  sessionID,
		RunID:      runID,
		Role:       messagestore.RoleCompaction,
		Text:       "local pruned context",
		RawJSON:    append([]byte(nil), messages[0].RawJSON...),
		SourceKind: "compacted",
	}); err != nil {
		return err
	}
	for _, msg := range messages[1:] {
		if err := appendStoredProviderMessage(ctx, a.Messages, sessionID, runID, msg); err != nil {
			return err
		}
	}
	return nil
}

func (a *App) appendPrunedItems(ctx context.Context, sessionID string, runID string, messages []provider.Message) error {
	if a.Items == nil || len(messages) == 0 {
		return nil
	}
	items := []session.Item{{
		ID:        runID + ":local-prune",
		SessionID: sessionID,
		TurnID:    runID,
		Kind:      session.ItemCompaction,
		Text:      provider.LocalPruneMarkerText,
		RawJSON:   append([]byte(nil), messages[0].RawJSON...),
	}}
	for i, msg := range messages[1:] {
		if item, ok := sessionItemFromProviderMessage(sessionID, runID, i, msg); ok {
			items = append(items, item)
		}
	}
	_, err := a.Items.AppendMany(ctx, items)
	return err
}

func (a *App) compactWithOrphanRepair(ctx context.Context, compactor provider.Compactor, sessionID string, runID string, messages []provider.Message) (provider.CompactResult, error) {
	attemptMessages := messages
	for repairs := 0; ; repairs++ {
		if repaired, callIDs, ok := provider.RepairAllOrphanFunctionCallOutputs(attemptMessages); ok {
			a.recordCompactionRepair(ctx, sessionID, runID, callIDs, len(attemptMessages)-len(repaired), "", false)
			attemptMessages = repaired
		}
		result, err := compactor.Compact(ctx, provider.CompactRequest{
			SessionID:    sessionID,
			RunID:        runID,
			Model:        a.Config.Model,
			Instructions: agent.GodeInstructions,
			Messages:     attemptMessages,
		})
		if err == nil {
			return result, nil
		}
		repaired, callID, ok := provider.RepairOrphanFunctionCallOutput(attemptMessages, err)
		if !ok || repairs >= 8 || len(repaired) == 0 {
			return provider.CompactResult{}, err
		}
		a.recordCompactionRepair(ctx, sessionID, runID, []string{callID}, len(attemptMessages)-len(repaired), err.Error(), true)
		attemptMessages = repaired
	}
}

func (a *App) recordCompactionRepair(ctx context.Context, sessionID string, runID string, callIDs []string, removed int, originalErr string, retry bool) {
	payload := map[string]any{
		"model":    a.Config.Model,
		"call_ids": callIDs,
		"removed":  removed,
		"repair":   "removed_orphan_function_call_output",
		"retry":    retry,
	}
	if len(callIDs) == 1 {
		payload["call_id"] = callIDs[0]
	}
	if originalErr != "" {
		payload["original_err"] = originalErr
	}
	ev := a.publish(ctx, eventbus.Event{
		Kind:      eventbus.KindContextCompactionRepaired,
		Source:    eventbus.SourceAgent,
		SessionID: sessionID,
		RunID:     runID,
		Payload:   payload,
	})
	a.appendJournal(ctx, ev)
}

func (a *App) recordLocalPruneRepair(ctx context.Context, sessionID string, runID string, dropped int, originalErr error) {
	ev := a.publish(ctx, eventbus.Event{
		Kind:      eventbus.KindContextCompactionRepaired,
		Source:    eventbus.SourceAgent,
		SessionID: sessionID,
		RunID:     runID,
		Payload: map[string]any{
			"model":        a.Config.Model,
			"dropped":      dropped,
			"repair":       "local_prune_after_compaction_failed",
			"original_err": originalErr.Error(),
		},
	})
	a.appendJournal(ctx, ev)
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

func appendStoredProviderMessage(ctx context.Context, store *messagestore.Store, sessionID string, runID string, msg provider.Message) error {
	if len(msg.RawJSON) > 0 {
		_, err := store.Append(ctx, messagestore.Message{SessionID: sessionID, RunID: runID, Role: messagestore.RoleCompaction, Text: "raw context item", RawJSON: append([]byte(nil), msg.RawJSON...), SourceKind: "compaction_suffix"})
		return err
	}
	switch msg.Role {
	case provider.RoleUser:
		_, err := store.Append(ctx, messagestore.Message{SessionID: sessionID, RunID: runID, Role: messagestore.RoleUser, Text: msg.Content, SourceKind: "compaction_suffix"})
		return err
	case provider.RoleAssistant:
		_, err := store.Append(ctx, messagestore.Message{SessionID: sessionID, RunID: runID, Role: messagestore.RoleAssistant, Text: msg.Content, SourceKind: "compaction_suffix"})
		return err
	case provider.RoleTool:
		_, err := store.Append(ctx, messagestore.Message{SessionID: sessionID, RunID: runID, Role: messagestore.RoleTool, Text: msg.Content, ToolCallID: msg.ToolCallID, ToolName: msg.ToolName, SourceKind: "compaction_suffix"})
		return err
	default:
		return nil
	}
}

func sessionItemFromProviderMessage(sessionID string, runID string, index int, msg provider.Message) (session.Item, bool) {
	item := session.Item{
		ID:        fmt.Sprintf("%s:local-prune:%d", runID, index),
		SessionID: sessionID,
		TurnID:    runID,
		Text:      msg.Content,
	}
	if len(msg.RawJSON) > 0 {
		item.Kind = session.ItemRaw
		item.RawJSON = append([]byte(nil), msg.RawJSON...)
		return item, true
	}
	switch msg.Role {
	case provider.RoleUser:
		item.Kind = session.ItemMessage
		item.Role = "user"
	case provider.RoleAssistant:
		item.Kind = session.ItemMessage
		item.Role = "assistant"
	case provider.RoleTool:
		item.Kind = session.ItemFunctionOut
		item.Role = "tool"
		item.ToolCallID = msg.ToolCallID
		item.ToolName = msg.ToolName
	default:
		return session.Item{}, false
	}
	return item, true
}

func providerMessagesFromSessionItems(items []session.Item) []provider.Message {
	items = canonicalSessionItemWindow(items)
	out := make([]provider.Message, 0, len(items))
	for _, item := range items {
		if len(item.RawJSON) > 0 && item.Kind != session.ItemFunctionCall && item.Kind != session.ItemFunctionOut {
			out = append(out, provider.Message{RawJSON: append([]byte(nil), item.RawJSON...)})
			continue
		}
		switch item.Kind {
		case session.ItemMessage:
			out = append(out, provider.Message{Role: provider.Role(item.Role), Phase: item.Phase, Content: item.Text, Images: providerImagesFromSessionImages(item.Images)})
		case session.ItemFunctionCall:
			out = append(out, provider.Message{
				Role:          provider.RoleAssistant,
				ToolCallID:    item.ToolCallID,
				ToolName:      item.ToolName,
				ToolArguments: item.Text,
			})
		case session.ItemFunctionOut:
			out = append(out, provider.Message{Role: provider.RoleTool, ToolCallID: item.ToolCallID, Content: item.Text})
		case session.ItemCompaction, session.ItemRaw:
			if len(item.RawJSON) > 0 {
				out = append(out, provider.Message{RawJSON: append([]byte(nil), item.RawJSON...)})
			} else if item.Text != "" {
				out = append(out, provider.Message{Role: provider.RoleUser, Content: item.Text})
			}
		}
	}
	return out
}

func canonicalSessionItemWindow(items []session.Item) []session.Item {
	latestCompaction := -1
	latestTurnID := ""
	for i, item := range items {
		if item.Kind == session.ItemCompaction {
			latestCompaction = i
			latestTurnID = item.TurnID
		}
	}
	if latestCompaction == -1 {
		return items
	}
	start := latestCompaction
	for i := latestCompaction; i >= 0; i-- {
		item := items[i]
		if item.Kind != session.ItemCompaction {
			break
		}
		if latestTurnID != "" && item.TurnID != latestTurnID {
			break
		}
		start = i
	}
	return items[start:]
}

func providerImagesFromSessionImages(images []session.Image) []provider.Image {
	if len(images) == 0 {
		return nil
	}
	out := make([]provider.Image, 0, len(images))
	for _, image := range images {
		out = append(out, provider.Image{URL: image.URL, Detail: image.Detail})
	}
	return out
}
