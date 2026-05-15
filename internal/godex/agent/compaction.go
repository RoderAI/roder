package agent

import (
	"context"
	"encoding/json"
	"fmt"

	"github.com/pandelisz/gode/internal/godex/contextwindow"
	"github.com/pandelisz/gode/internal/godex/eventbus"
	messagestore "github.com/pandelisz/gode/internal/godex/message"
	"github.com/pandelisz/gode/internal/godex/provider"
	"github.com/pandelisz/gode/internal/godex/session"
)

func (r *Runner) compactContextIfNeeded(ctx context.Context, req RunRequest, messages []provider.Message, inputItems []provider.Item, tools []provider.ToolSpec) ([]provider.Message, []provider.Item, error) {
	return r.compactContext(ctx, req, messages, inputItems, tools, false, "")
}

func (r *Runner) forceCompactContext(ctx context.Context, req RunRequest, messages []provider.Message, inputItems []provider.Item, tools []provider.ToolSpec, reason string) ([]provider.Message, []provider.Item, error) {
	return r.compactContext(ctx, req, messages, inputItems, tools, true, reason)
}

func (r *Runner) repairOrphanToolOutputs(ctx context.Context, req RunRequest, messages []provider.Message, inputItems []provider.Item, err error, retry bool) ([]provider.Message, []provider.Item, bool) {
	model := firstNonEmpty(r.model, "gpt-5.5")
	repairedMessages := messages
	repairedInputItems := inputItems
	var callIDs []string
	removed := 0
	if err != nil {
		if repaired, callID, ok := provider.RepairOrphanFunctionCallOutput(messages, err); ok {
			removed += len(messages) - len(repaired)
			repairedMessages = repaired
			callIDs = append(callIDs, callID)
		}
		if repaired, callID, ok := provider.RepairOrphanFunctionCallOutputItems(inputItems, err); ok {
			removed += len(inputItems) - len(repaired)
			repairedInputItems = repaired
			callIDs = append(callIDs, callID)
		}
	} else {
		if repaired, ids, ok := provider.RepairAllOrphanFunctionCallOutputs(messages); ok {
			removed += len(messages) - len(repaired)
			repairedMessages = repaired
			callIDs = append(callIDs, ids...)
		}
		if repaired, ids, ok := provider.RepairAllOrphanFunctionCallOutputItems(inputItems); ok {
			removed += len(inputItems) - len(repaired)
			repairedInputItems = repaired
			callIDs = append(callIDs, ids...)
		}
	}
	callIDs = uniqueStrings(callIDs)
	if removed == 0 {
		return messages, inputItems, false
	}
	originalErr := ""
	if err != nil {
		originalErr = err.Error()
	}
	r.emitCompactionRepair(ctx, req, model, callIDs, removed, originalErr, retry)
	r.persistPrunedWindow(ctx, req, append([]provider.Message{provider.LocalPruneMarkerMessage()}, repairedMessages...))
	return repairedMessages, repairedInputItems, true
}

func (r *Runner) compactContext(ctx context.Context, req RunRequest, messages []provider.Message, inputItems []provider.Item, tools []provider.ToolSpec, force bool, reason string) ([]provider.Message, []provider.Item, error) {
	model := firstNonEmpty(r.model, "gpt-5.5")
	options := contextwindow.OptionsForModel(model, r.disableAutoCompaction, r.autoCompactTokenLimit)
	if r.shouldUseProviderNeutralCompaction(model, options) {
		return r.compactContextWithProviderSummary(ctx, req, messages, inputItems, tools, options, force, reason)
	}
	if !options.Enabled || r.providerName() != "openai" {
		return messages, inputItems, nil
	}
	estimate := r.requestTokenEstimate(req, messages, inputItems, tools)
	if !force && estimate.Tokens < options.CompactThreshold {
		return messages, inputItems, nil
	}
	compactor, ok := r.provider.(provider.Compactor)
	if !ok {
		return messages, inputItems, nil
	}

	compactable, suffix := splitCompactionWindow(messages)
	if len(compactable) == 0 {
		return messages, inputItems, nil
	}
	r.emit(ctx, eventbus.Event{
		Kind:      eventbus.KindContextCompactionStarted,
		Source:    eventbus.SourceAgent,
		SessionID: req.SessionID,
		RunID:     req.RunID,
		Payload: map[string]any{
			"model":             model,
			"tokens":            estimate.Tokens,
			"context_window":    options.ContextWindow,
			"compact_threshold": options.CompactThreshold,
		},
	})
	if force && reason != "" {
		payload := map[string]any{"reason": reason}
		r.emit(ctx, eventbus.Event{
			Kind:      eventbus.KindContextCompactionRepaired,
			Source:    eventbus.SourceAgent,
			SessionID: req.SessionID,
			RunID:     req.RunID,
			Payload:   payload,
		})
	}

	attemptMessages := compactable
	var result provider.CompactResult
	var err error
	for repairs := 0; ; repairs++ {
		if repaired, callIDs, ok := provider.RepairAllOrphanFunctionCallOutputs(attemptMessages); ok {
			r.emitCompactionRepair(ctx, req, model, callIDs, len(attemptMessages)-len(repaired), "", false)
			attemptMessages = repaired
		}
		result, err = compactor.Compact(ctx, provider.CompactRequest{
			SessionID:      req.SessionID,
			RunID:          req.RunID,
			Model:          options.Model,
			PromptCacheKey: r.promptCacheKey(),
			Instructions:   firstNonEmpty(req.Instructions, GodeInstructions),
			Messages:       attemptMessages,
		})
		if err == nil {
			compactable = attemptMessages
			break
		}
		repaired, callID, ok := provider.RepairOrphanFunctionCallOutput(attemptMessages, err)
		if !ok || repairs >= 8 {
			break
		}
		if len(repaired) == 0 {
			r.emitCompactionRepair(ctx, req, model, []string{callID}, len(attemptMessages)-len(repaired), err.Error(), true)
			next := append([]provider.Message{}, suffix...)
			return next, providerItemsFromProviderMessages(next), nil
		}
		r.emitCompactionRepair(ctx, req, model, []string{callID}, len(attemptMessages)-len(repaired), err.Error(), true)
		attemptMessages = repaired
	}
	if err != nil {
		if provider.ShouldPruneAfterCompactionError(err) {
			pruned, dropped, ok := provider.LocalPrunedMessages(provider.LocalPruneRequest{
				Model:        model,
				Instructions: firstNonEmpty(req.Instructions, GodeInstructions),
				Messages:     append(append([]provider.Message{}, attemptMessages...), suffix...),
				Tools:        tools,
				TargetTokens: options.CompactThreshold,
			})
			if ok {
				r.emitLocalPruneRepair(ctx, req, model, dropped, err)
				r.persistPrunedWindow(ctx, req, pruned)
				return pruned, providerItemsFromProviderMessages(pruned), nil
			}
		}
		r.emit(ctx, eventbus.Event{
			Kind:      eventbus.KindContextCompactionFailed,
			Source:    eventbus.SourceAgent,
			SessionID: req.SessionID,
			RunID:     req.RunID,
			Payload:   map[string]any{"model": model, "error": err.Error()},
		})
		return nil, nil, err
	}
	compacted := rawCompactionMessages(result.Output)
	if len(compacted) == 0 {
		err := fmt.Errorf("compaction returned no output items")
		r.emit(ctx, eventbus.Event{
			Kind:      eventbus.KindContextCompactionFailed,
			Source:    eventbus.SourceAgent,
			SessionID: req.SessionID,
			RunID:     req.RunID,
			Payload:   map[string]any{"model": model, "error": err.Error()},
		})
		return nil, nil, err
	}
	r.persistCompactedWindow(ctx, req, result.Output, suffix)
	r.emit(ctx, eventbus.Event{
		Kind:      eventbus.KindContextCompactionCompleted,
		Source:    eventbus.SourceAgent,
		SessionID: req.SessionID,
		RunID:     req.RunID,
		Payload: map[string]any{
			"model":        model,
			"response_id":  result.ID,
			"output_items": len(result.Output),
		},
	})
	next := append([]provider.Message{}, compacted...)
	next = append(next, suffix...)
	return next, providerItemsFromProviderMessages(next), nil
}

func uniqueStrings(values []string) []string {
	seen := map[string]bool{}
	out := make([]string, 0, len(values))
	for _, value := range values {
		if value == "" || seen[value] {
			continue
		}
		seen[value] = true
		out = append(out, value)
	}
	return out
}

func (r *Runner) emitLocalPruneRepair(ctx context.Context, req RunRequest, model string, dropped int, originalErr error) {
	r.emit(ctx, eventbus.Event{
		Kind:      eventbus.KindContextCompactionRepaired,
		Source:    eventbus.SourceAgent,
		SessionID: req.SessionID,
		RunID:     req.RunID,
		Payload: map[string]any{
			"model":        model,
			"dropped":      dropped,
			"repair":       "local_prune_after_compaction_failed",
			"original_err": originalErr.Error(),
		},
	})
}

func (r *Runner) emitCompactionRepair(ctx context.Context, req RunRequest, model string, callIDs []string, removed int, originalErr string, retry bool) {
	payload := map[string]any{
		"model":    model,
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
	r.emit(ctx, eventbus.Event{
		Kind:      eventbus.KindContextCompactionRepaired,
		Source:    eventbus.SourceAgent,
		SessionID: req.SessionID,
		RunID:     req.RunID,
		Payload:   payload,
	})
}

func splitCompactionWindow(messages []provider.Message) ([]provider.Message, []provider.Message) {
	if len(messages) <= 1 {
		return messages, nil
	}
	last := messages[len(messages)-1]
	if last.Role == provider.RoleUser && len(last.RawJSON) == 0 {
		return messages[:len(messages)-1], messages[len(messages)-1:]
	}
	return messages, nil
}

func rawCompactionMessages(items []json.RawMessage) []provider.Message {
	out := make([]provider.Message, 0, len(items))
	for _, item := range items {
		if len(item) == 0 {
			continue
		}
		out = append(out, provider.Message{RawJSON: append([]byte(nil), item...)})
	}
	return out
}

func (r *Runner) persistPrunedWindow(ctx context.Context, req RunRequest, messages []provider.Message) {
	if len(messages) == 0 {
		return
	}
	if r.items != nil {
		items := []session.Item{{
			ID:        req.RunID + ":local-prune",
			SessionID: req.SessionID,
			TurnID:    req.RunID,
			Kind:      session.ItemCompaction,
			Text:      provider.LocalPruneMarkerText,
			RawJSON:   append([]byte(nil), messages[0].RawJSON...),
		}}
		for i, msg := range messages[1:] {
			if item, ok := providerSuffixItem(req, msg, i); ok {
				items = append(items, item)
			}
		}
		if len(items) > 0 {
			_, _ = r.items.AppendMany(ctx, items)
		}
	}
	if r.messages != nil {
		_, _ = r.messages.Append(ctx, messagestore.Message{
			SessionID:  req.SessionID,
			RunID:      req.RunID,
			Role:       messagestore.RoleCompaction,
			Text:       "local pruned context",
			RawJSON:    append([]byte(nil), messages[0].RawJSON...),
			SourceKind: "compacted",
		})
		for _, msg := range messages[1:] {
			_ = appendProviderSuffix(ctx, r.messages, req, msg)
		}
	}
}

func (r *Runner) persistCompactedWindow(ctx context.Context, req RunRequest, items []json.RawMessage, suffix []provider.Message) {
	if r.items != nil {
		var stored []session.Item
		for i, raw := range items {
			stored = append(stored, session.Item{
				ID:        fmt.Sprintf("%s:compaction:%d", req.RunID, i),
				SessionID: req.SessionID,
				TurnID:    req.RunID,
				Kind:      session.ItemCompaction,
				RawJSON:   append([]byte(nil), raw...),
			})
		}
		for i, msg := range suffix {
			if item, ok := providerSuffixItem(req, msg, i); ok {
				stored = append(stored, item)
			}
		}
		if len(stored) > 0 {
			_, _ = r.items.AppendMany(ctx, stored)
		}
	}
	if r.messages != nil {
		for i, raw := range items {
			text := "compaction item"
			if i == len(items)-1 {
				text = "canonical compacted context"
			}
			_, _ = r.messages.Append(ctx, messagestore.Message{
				SessionID:  req.SessionID,
				RunID:      req.RunID,
				Role:       messagestore.RoleCompaction,
				Text:       text,
				RawJSON:    append([]byte(nil), raw...),
				SourceKind: "compacted",
			})
		}
		for _, msg := range suffix {
			_ = appendProviderSuffix(ctx, r.messages, req, msg)
		}
	}
}

func providerSuffixItem(req RunRequest, msg provider.Message, index int) (session.Item, bool) {
	item := session.Item{
		ID:        fmt.Sprintf("%s:suffix:%d", req.RunID, index),
		SessionID: req.SessionID,
		TurnID:    req.RunID,
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

func appendProviderSuffix(ctx context.Context, store *messagestore.Store, req RunRequest, msg provider.Message) error {
	if len(msg.RawJSON) > 0 {
		_, err := store.Append(ctx, messagestore.Message{SessionID: req.SessionID, RunID: req.RunID, Role: messagestore.RoleCompaction, Text: "raw context item", RawJSON: append([]byte(nil), msg.RawJSON...), SourceKind: "compaction_suffix"})
		return err
	}
	switch msg.Role {
	case provider.RoleUser:
		_, err := store.Append(ctx, messagestore.Message{SessionID: req.SessionID, RunID: req.RunID, Role: messagestore.RoleUser, Text: msg.Content, SourceKind: "compaction_suffix"})
		return err
	case provider.RoleAssistant:
		_, err := store.Append(ctx, messagestore.Message{SessionID: req.SessionID, RunID: req.RunID, Role: messagestore.RoleAssistant, Text: msg.Content, SourceKind: "compaction_suffix"})
		return err
	case provider.RoleTool:
		_, err := store.Append(ctx, messagestore.Message{SessionID: req.SessionID, RunID: req.RunID, Role: messagestore.RoleTool, Text: msg.Content, ToolCallID: msg.ToolCallID, ToolName: msg.ToolName, SourceKind: "compaction_suffix"})
		return err
	default:
		return nil
	}
}

func (r *Runner) compactionOptions(ctx context.Context, req RunRequest, messages []provider.Message, inputItems []provider.Item, tools []provider.ToolSpec) provider.CompactionOptions {
	model := firstNonEmpty(r.model, "gpt-5.5")
	estimate := r.requestTokenEstimate(req, messages, inputItems, tools)
	r.emit(ctx, eventbus.Event{
		Kind:      eventbus.KindContextTokensUpdated,
		Source:    eventbus.SourceAgent,
		SessionID: req.SessionID,
		RunID:     req.RunID,
		Payload: map[string]any{
			"model":          model,
			"tokens":         estimate.Tokens,
			"context_window": estimate.ContextWindow,
			"percent":        estimate.Percent,
		},
	})

	options := contextwindow.OptionsForModel(model, r.disableAutoCompaction, r.autoCompactTokenLimit)
	if r.shouldUseProviderNeutralCompaction(model, options) {
		r.emit(ctx, eventbus.Event{
			Kind:      eventbus.KindContextCompactionConfigured,
			Source:    eventbus.SourceAgent,
			SessionID: req.SessionID,
			RunID:     req.RunID,
			Payload: map[string]any{
				"model":             model,
				"tokens":            estimate.Tokens,
				"context_window":    options.ContextWindow,
				"compact_threshold": options.CompactThreshold,
				"strategy":          "provider_neutral_summary",
			},
		})
		return provider.CompactionOptions{
			Model:            options.Model,
			ContextWindow:    options.ContextWindow,
			CompactThreshold: options.CompactThreshold,
		}
	}
	if !options.Enabled || r.providerName() != "openai" {
		return provider.CompactionOptions{
			Model:            options.Model,
			ContextWindow:    options.ContextWindow,
			CompactThreshold: options.CompactThreshold,
		}
	}
	r.emit(ctx, eventbus.Event{
		Kind:      eventbus.KindContextCompactionConfigured,
		Source:    eventbus.SourceAgent,
		SessionID: req.SessionID,
		RunID:     req.RunID,
		Payload: map[string]any{
			"model":             model,
			"tokens":            estimate.Tokens,
			"context_window":    options.ContextWindow,
			"compact_threshold": options.CompactThreshold,
		},
	})
	return provider.CompactionOptions{
		Enabled:          true,
		Model:            options.Model,
		ContextWindow:    options.ContextWindow,
		CompactThreshold: options.CompactThreshold,
	}
}

func (r *Runner) emitActualTokenUsage(ctx context.Context, req RunRequest, usage provider.TokenUsage) {
	if usage.IsZero() {
		return
	}
	model := firstNonEmpty(r.model, "gpt-5.5")
	window := contextwindow.ForModel(model)
	percent := 0.0
	if window.ContextWindow > 0 {
		percent = float64(usage.Total()) / float64(window.ContextWindow) * 100
	}
	r.emit(ctx, eventbus.Event{
		Kind:      eventbus.KindContextTokensUpdated,
		Source:    eventbus.SourceProvider,
		SessionID: req.SessionID,
		RunID:     req.RunID,
		Payload: map[string]any{
			"model":           model,
			"tokens":          usage.Total(),
			"input_tokens":    usage.InputTokens,
			"output_tokens":   usage.OutputTokens,
			"total_tokens":    usage.Total(),
			"context_window":  window.ContextWindow,
			"percent":         percent,
			"count_source":    "response",
			"usage_increment": true,
		},
	})
}

func (r *Runner) requestTokenEstimate(req RunRequest, messages []provider.Message, inputItems []provider.Item, tools []provider.ToolSpec) contextwindow.TokenEstimate {
	model := firstNonEmpty(r.model, "gpt-5.5")
	return contextwindow.EstimateRequest(contextwindow.Request{
		Model:          model,
		Instructions:   firstNonEmpty(req.Instructions, GodeInstructions),
		ResponseFormat: req.ResponseFormat,
		Messages:       contextWindowMessages(messages),
		InputItems:     contextWindowItems(inputItems),
		Tools:          contextWindowToolSpecs(tools),
	}, contextwindow.ForModel(model))
}

func contextWindowMessages(messages []provider.Message) []contextwindow.Message {
	out := make([]contextwindow.Message, 0, len(messages))
	for _, msg := range messages {
		out = append(out, contextwindow.Message{
			Role:          string(msg.Role),
			Content:       msg.Content,
			Phase:         msg.Phase,
			Images:        contextWindowImages(msg.Images),
			ToolCallID:    msg.ToolCallID,
			ToolName:      msg.ToolName,
			ToolArguments: msg.ToolArguments,
			RawJSON:       msg.RawJSON,
		})
	}
	return out
}

func contextWindowItems(items []provider.Item) []contextwindow.Item {
	out := make([]contextwindow.Item, 0, len(items))
	for _, item := range items {
		out = append(out, contextwindow.Item{
			Kind:       string(item.Kind),
			Role:       item.Role,
			Phase:      item.Phase,
			ToolName:   item.ToolName,
			ToolCallID: item.ToolCallID,
			Text:       item.Text,
			Images:     contextWindowImages(item.Images),
			RawJSON:    item.RawJSON,
		})
	}
	return out
}

func contextWindowToolSpecs(tools []provider.ToolSpec) []contextwindow.ToolSpec {
	out := make([]contextwindow.ToolSpec, 0, len(tools))
	for _, tool := range tools {
		out = append(out, contextwindow.ToolSpec{
			Name:        tool.Name,
			Description: tool.Description,
			Schema:      tool.Schema,
		})
	}
	return out
}

func contextWindowImages(images []provider.Image) []contextwindow.Image {
	out := make([]contextwindow.Image, 0, len(images))
	for _, image := range images {
		out = append(out, contextwindow.Image{URL: image.URL, Detail: image.Detail})
	}
	return out
}
