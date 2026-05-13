package agent

import (
	"context"

	"github.com/google/uuid"
	"github.com/pandelisz/gode/internal/godex/provider"
	"github.com/pandelisz/gode/internal/godex/session"
)

func (r *Runner) startTurn(ctx context.Context, req RunRequest) error {
	if r.turns == nil {
		return nil
	}
	_, err := r.turns.Append(ctx, session.Turn{
		ID:        req.RunID,
		SessionID: req.SessionID,
		Prompt:    req.Prompt,
		Model:     r.model,
		Provider:  r.providerName(),
		Status:    session.TurnStatusRunning,
	})
	return err
}

func (r *Runner) completeTurn(ctx context.Context, req RunRequest, _ string, responseID string) error {
	if r.turns == nil {
		return nil
	}
	_, err := r.turns.Complete(ctx, req.SessionID, req.RunID, responseID)
	return err
}

func (r *Runner) failTurn(ctx context.Context, req RunRequest, errorText string) error {
	if r.turns == nil {
		return nil
	}
	_, err := r.turns.Fail(ctx, req.SessionID, req.RunID, errorText)
	return err
}

func (r *Runner) persistUserItem(ctx context.Context, req RunRequest) error {
	if r.items == nil {
		return nil
	}
	_, err := r.items.Append(ctx, session.Item{
		ID:        req.RunID + ":user",
		SessionID: req.SessionID,
		TurnID:    req.RunID,
		Kind:      session.ItemMessage,
		Role:      "user",
		Text:      req.Prompt,
	})
	return err
}

func (r *Runner) persistProviderItems(ctx context.Context, req RunRequest, providerItems []provider.Item, final string) error {
	if r.items == nil {
		return nil
	}
	items := r.sessionItemsFromProviderItems(req, providerItems)
	if len(items) == 0 && final != "" {
		items = append(items, session.Item{
			ID:        req.RunID + ":assistant",
			SessionID: req.SessionID,
			TurnID:    req.RunID,
			Kind:      session.ItemMessage,
			Role:      "assistant",
			Text:      final,
		})
	}
	if len(items) == 0 {
		return nil
	}
	_, err := r.items.AppendMany(ctx, items)
	return err
}

func (r *Runner) sessionItemsFromProviderItems(req RunRequest, providerItems []provider.Item) []session.Item {
	items := make([]session.Item, 0, len(providerItems))
	for _, item := range providerItems {
		items = append(items, session.Item{
			ID:         firstNonEmpty(item.ID, uuid.NewString()),
			SessionID:  req.SessionID,
			TurnID:     req.RunID,
			Kind:       sessionItemKind(item.Kind),
			Role:       item.Role,
			ToolName:   item.ToolName,
			ToolCallID: item.ToolCallID,
			Text:       item.Text,
			RawJSON:    append([]byte(nil), item.RawJSON...),
		})
	}
	return items
}

func providerMessagesFromSessionItems(items []session.Item) []provider.Message {
	items = canonicalProviderItems(items)
	out := make([]provider.Message, 0, len(items))
	for _, item := range items {
		out = appendProviderMessageFromSessionItem(out, item)
	}
	return out
}

func providerItemsFromSessionItems(items []session.Item) []provider.Item {
	items = canonicalProviderItems(items)
	out := make([]provider.Item, 0, len(items))
	for _, item := range items {
		out = append(out, provider.Item{
			ID:         item.ID,
			Kind:       providerItemKind(item.Kind),
			Role:       item.Role,
			ToolName:   item.ToolName,
			ToolCallID: item.ToolCallID,
			Text:       item.Text,
			RawJSON:    append([]byte(nil), item.RawJSON...),
		})
	}
	return out
}

func providerMessagesFromProviderItems(items []provider.Item) []provider.Message {
	out := make([]provider.Message, 0, len(items))
	for _, item := range items {
		out = appendProviderMessageFromSessionItem(out, session.Item{
			Kind:       sessionItemKind(item.Kind),
			Role:       item.Role,
			ToolName:   item.ToolName,
			ToolCallID: item.ToolCallID,
			Text:       item.Text,
			RawJSON:    item.RawJSON,
		})
	}
	return out
}

func providerItemsFromProviderMessages(messages []provider.Message) []provider.Item {
	out := make([]provider.Item, 0, len(messages))
	for _, message := range messages {
		item := providerItemFromProviderMessage(message)
		if item.Kind != "" {
			out = append(out, item)
		}
	}
	return out
}

func providerItemFromProviderMessage(message provider.Message) provider.Item {
	if len(message.RawJSON) > 0 {
		return provider.Item{Kind: provider.ItemRaw, RawJSON: append([]byte(nil), message.RawJSON...)}
	}
	switch {
	case message.Role == provider.RoleAssistant && message.ToolCallID != "" && message.ToolName != "":
		return provider.Item{Kind: provider.ItemFunctionCall, ToolName: message.ToolName, ToolCallID: message.ToolCallID, Text: message.ToolArguments}
	case message.Role == provider.RoleTool && message.ToolCallID != "":
		return provider.Item{Kind: provider.ItemFunctionOut, Role: string(provider.RoleTool), ToolName: message.ToolName, ToolCallID: message.ToolCallID, Text: message.Content}
	case message.Role != "":
		return provider.Item{Kind: provider.ItemMessage, Role: string(message.Role), Text: message.Content}
	default:
		return provider.Item{}
	}
}

func canonicalProviderItems(items []session.Item) []session.Item {
	latestCompaction := -1
	latestTurnID := ""
	for i, item := range items {
		if item.Kind == session.ItemCompaction && len(item.RawJSON) > 0 {
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
		if item.Kind != session.ItemCompaction || len(item.RawJSON) == 0 {
			break
		}
		if latestTurnID != "" && item.TurnID != latestTurnID {
			break
		}
		start = i
	}
	return items[start:]
}

func appendProviderMessageFromSessionItem(out []provider.Message, item session.Item) []provider.Message {
	if len(item.RawJSON) > 0 && item.Kind != session.ItemFunctionCall && item.Kind != session.ItemFunctionOut {
		return append(out, provider.Message{RawJSON: append([]byte(nil), item.RawJSON...)})
	}
	switch item.Kind {
	case session.ItemMessage:
		out = append(out, provider.Message{Role: provider.Role(item.Role), Content: item.Text})
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
		}
	}
	return out
}

func excludeRunItems(items []session.Item, runID string) []session.Item {
	if runID == "" {
		return items
	}
	out := items[:0]
	for _, item := range items {
		if item.TurnID != runID {
			out = append(out, item)
		}
	}
	return out
}

func sessionItemKind(kind provider.ItemKind) session.ItemKind {
	switch kind {
	case provider.ItemMessage:
		return session.ItemMessage
	case provider.ItemFunctionCall:
		return session.ItemFunctionCall
	case provider.ItemFunctionOut:
		return session.ItemFunctionOut
	case provider.ItemReasoning:
		return session.ItemReasoning
	case provider.ItemCompaction:
		return session.ItemCompaction
	default:
		return session.ItemRaw
	}
}

func providerItemKind(kind session.ItemKind) provider.ItemKind {
	switch kind {
	case session.ItemMessage:
		return provider.ItemMessage
	case session.ItemFunctionCall:
		return provider.ItemFunctionCall
	case session.ItemFunctionOut:
		return provider.ItemFunctionOut
	case session.ItemReasoning:
		return provider.ItemReasoning
	case session.ItemCompaction:
		return provider.ItemCompaction
	default:
		return provider.ItemRaw
	}
}
