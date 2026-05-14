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

func (r *Runner) compactContextIfNeeded(ctx context.Context, req RunRequest, messages []provider.Message, inputItems []provider.Item) ([]provider.Message, []provider.Item, error) {
	model := firstNonEmpty(r.model, "gpt-5.5")
	options := contextwindow.OptionsForModel(model, r.disableAutoCompaction, r.autoCompactTokenLimit)
	if !options.Enabled || r.providerName() != "openai" {
		return messages, inputItems, nil
	}
	estimate := contextwindow.EstimateMessages(contextWindowMessages(messages), contextwindow.ForModel(model))
	if estimate.Tokens < options.CompactThreshold {
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

	result, err := compactor.Compact(ctx, provider.CompactRequest{
		SessionID:      req.SessionID,
		RunID:          req.RunID,
		Model:          options.Model,
		PromptCacheKey: r.promptCacheKey(),
		Instructions:   firstNonEmpty(req.Instructions, GodeInstructions),
		Messages:       compactable,
	})
	if err != nil {
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

func (r *Runner) compactionOptions(ctx context.Context, req RunRequest, messages []provider.Message) provider.CompactionOptions {
	model := firstNonEmpty(r.model, "gpt-5.5")
	window := contextwindow.ForModel(model)
	estimate := contextwindow.EstimateMessages(contextWindowMessages(messages), window)
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

func contextWindowMessages(messages []provider.Message) []contextwindow.Message {
	out := make([]contextwindow.Message, 0, len(messages))
	for _, msg := range messages {
		out = append(out, contextwindow.Message{
			Role:          string(msg.Role),
			Content:       msg.Content,
			ToolCallID:    msg.ToolCallID,
			ToolName:      msg.ToolName,
			ToolArguments: msg.ToolArguments,
			RawJSON:       msg.RawJSON,
		})
	}
	return out
}
