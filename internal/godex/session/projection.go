package session

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"time"

	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/journal"
)

type ProjectedPublisher interface {
	Publish(context.Context, eventbus.Event) eventbus.Event
}

type BackfillStores struct {
	Sessions *Store
	Turns    *TurnStore
	Items    *ItemStore
	Bus      ProjectedPublisher
}

type BackfillResult struct {
	Sessions int
	Turns    int
	Items    int
}

func ProjectionFromEvent(ev eventbus.Event) []Item {
	base := Item{
		ID:        ev.ID,
		SessionID: ev.SessionID,
		TurnID:    ev.RunID,
		CreatedAt: ev.Time,
	}
	switch ev.Kind {
	case eventbus.KindUserPromptSubmitted:
		var payload struct {
			Prompt string `json:"prompt"`
		}
		_ = ev.DecodePayload(&payload)
		return itemSingle(base, ItemMessage, "user", strings.TrimSpace(payload.Prompt))
	case eventbus.KindAssistantDelta, eventbus.KindAssistantCompleted:
		if items := payloadItems(ev); len(items) > 0 {
			return items
		}
		var payload struct {
			Text string `json:"text"`
		}
		_ = ev.DecodePayload(&payload)
		return itemSingle(base, ItemMessage, "assistant", strings.TrimSpace(payload.Text))
	case eventbus.KindReasoningSummaryDelta, eventbus.KindReasoningSummaryCompleted:
		var payload struct {
			Text string `json:"text"`
		}
		_ = ev.DecodePayload(&payload)
		return itemSingle(base, ItemReasoning, "", strings.TrimSpace(payload.Text))
	case eventbus.KindToolRequested:
		var payload struct {
			Tool       string         `json:"tool"`
			ToolCallID string         `json:"tool_call_id"`
			Input      map[string]any `json:"input"`
		}
		_ = ev.DecodePayload(&payload)
		base.ToolName = payload.Tool
		base.ToolCallID = payload.ToolCallID
		base.RawJSON = rawObject(map[string]any{
			"type":      string(ItemFunctionCall),
			"call_id":   payload.ToolCallID,
			"name":      payload.Tool,
			"arguments": payload.Input,
		})
		return itemSingle(base, ItemFunctionCall, "", string(base.RawJSON))
	case eventbus.KindToolCompleted:
		var payload struct {
			Tool       string `json:"tool"`
			ToolCallID string `json:"tool_call_id"`
			Text       string `json:"text"`
		}
		_ = ev.DecodePayload(&payload)
		base.ToolName = payload.Tool
		base.ToolCallID = payload.ToolCallID
		return itemSingle(base, ItemFunctionOut, "", strings.TrimSpace(payload.Text))
	case eventbus.KindToolFailed:
		var payload struct {
			Tool       string `json:"tool"`
			ToolCallID string `json:"tool_call_id"`
			Error      string `json:"error"`
			Text       string `json:"text"`
		}
		_ = ev.DecodePayload(&payload)
		base.ToolName = payload.Tool
		base.ToolCallID = payload.ToolCallID
		text := strings.TrimSpace(payload.Text)
		if text == "" {
			text = strings.TrimSpace(payload.Error)
		}
		return itemSingle(base, ItemFunctionOut, "", text)
	case eventbus.KindContextCompactionCompleted:
		var payload struct {
			OutputItems int `json:"output_items"`
		}
		_ = ev.DecodePayload(&payload)
		text := ""
		if payload.OutputItems > 0 {
			text = fmt.Sprintf("%d output items", payload.OutputItems)
		}
		base.RawJSON = rawObject(map[string]any{"type": string(ItemCompaction), "output_items": payload.OutputItems})
		return itemSingle(base, ItemCompaction, "", text)
	default:
		return nil
	}
}

func Backfill(ctx context.Context, journalStore *journal.Store, stores BackfillStores) (BackfillResult, error) {
	if journalStore == nil {
		return BackfillResult{}, fmt.Errorf("journal store is required")
	}
	if stores.Sessions == nil || stores.Turns == nil || stores.Items == nil {
		return BackfillResult{}, fmt.Errorf("session, turn, and item stores are required")
	}
	events, err := journalStore.Replay(ctx, journal.ReplayFilter{})
	if err != nil {
		return BackfillResult{}, err
	}

	result := BackfillResult{}
	seenSessions := map[string]bool{}
	turnsBySession := map[string]map[string]Turn{}
	itemsBySession := map[string]map[string]bool{}
	assistant := map[string]Item{}

	for _, ev := range events {
		if ev.SessionID == "" {
			continue
		}
		if err := ctx.Err(); err != nil {
			return result, err
		}
		if !seenSessions[ev.SessionID] {
			created, err := ensureProjectedSession(ctx, stores.Sessions, ev)
			if err != nil {
				return result, err
			}
			seenSessions[ev.SessionID] = true
			if created {
				result.Sessions++
			}
		}
		if err := projectTurn(ctx, stores.Turns, turnsBySession, ev, &result); err != nil {
			return result, err
		}
		if err := projectItems(ctx, stores.Items, itemsBySession, assistant, ev, &result); err != nil {
			return result, err
		}
	}
	for _, item := range assistant {
		if item.Text == "" {
			continue
		}
		appended, err := appendProjectedItem(ctx, stores.Items, itemsBySession, item)
		if err != nil {
			return result, err
		}
		if appended {
			result.Items++
		}
	}
	if stores.Bus != nil {
		stores.Bus.Publish(ctx, eventbus.Event{
			Source: eventbus.SourceSystem,
			Kind:   eventbus.KindSessionProjected,
			Payload: map[string]any{
				"sessions": result.Sessions,
				"turns":    result.Turns,
				"items":    result.Items,
			},
		})
	}
	return result, nil
}

func ensureProjectedSession(ctx context.Context, store *Store, ev eventbus.Event) (bool, error) {
	if _, ok, err := store.Get(ctx, ev.SessionID); err != nil {
		return false, err
	} else if ok {
		return false, nil
	}
	session := Session{
		ID:        ev.SessionID,
		Title:     titleFromEvent(ev),
		CreatedAt: eventTime(ev),
		UpdatedAt: eventTime(ev),
	}
	_, err := store.Ensure(ctx, session)
	return err == nil, err
}

func projectTurn(ctx context.Context, store *TurnStore, cache map[string]map[string]Turn, ev eventbus.Event, result *BackfillResult) error {
	if ev.RunID == "" {
		return nil
	}
	turns, err := cachedTurns(ctx, store, cache, ev.SessionID)
	if err != nil {
		return err
	}
	existing, hasTurn := turns[ev.RunID]
	switch ev.Kind {
	case eventbus.KindRunStarted, eventbus.KindUserPromptSubmitted:
		if hasTurn {
			return nil
		}
		turn := Turn{
			ID:        ev.RunID,
			SessionID: ev.SessionID,
			Prompt:    promptFromEvent(ev),
			Status:    TurnStatusRunning,
			StartedAt: eventTime(ev),
			UpdatedAt: eventTime(ev),
		}
		saved, err := store.Append(ctx, turn)
		if err != nil {
			return err
		}
		turns[saved.ID] = saved
		result.Turns++
	case eventbus.KindRunCompleted:
		if !hasTurn {
			saved, err := store.Append(ctx, Turn{ID: ev.RunID, SessionID: ev.SessionID, Status: TurnStatusRunning, StartedAt: eventTime(ev), UpdatedAt: eventTime(ev)})
			if err != nil {
				return err
			}
			turns[saved.ID] = saved
			result.Turns++
		}
		if existing.Status == TurnStatusCompleted {
			return nil
		}
		updated, err := store.Complete(ctx, ev.SessionID, ev.RunID, "")
		if err != nil {
			return err
		}
		turns[updated.ID] = updated
	case eventbus.KindRunFailed:
		if !hasTurn {
			saved, err := store.Append(ctx, Turn{ID: ev.RunID, SessionID: ev.SessionID, Status: TurnStatusRunning, StartedAt: eventTime(ev), UpdatedAt: eventTime(ev)})
			if err != nil {
				return err
			}
			turns[saved.ID] = saved
			result.Turns++
		}
		if existing.Status == TurnStatusFailed {
			return nil
		}
		updated, err := store.Fail(ctx, ev.SessionID, ev.RunID, runErrorFromEvent(ev))
		if err != nil {
			return err
		}
		turns[updated.ID] = updated
	}
	return nil
}

func projectItems(ctx context.Context, store *ItemStore, cache map[string]map[string]bool, assistant map[string]Item, ev eventbus.Event, result *BackfillResult) error {
	items := ProjectionFromEvent(ev)
	for _, item := range items {
		if (ev.Kind == eventbus.KindAssistantDelta || ev.Kind == eventbus.KindAssistantCompleted) && item.Kind == ItemMessage && item.Role == "assistant" {
			key := ev.SessionID + "\x00" + ev.RunID
			existing := assistant[key]
			if existing.ID == "" {
				existing = item
				existing.ID = firstNonEmpty(ev.RunID, ev.ID) + ":assistant"
			} else if ev.Kind == eventbus.KindAssistantCompleted && item.Text != "" {
				existing.Text = item.Text
			} else {
				existing.Text += item.Text
			}
			if item.CreatedAt.After(existing.CreatedAt) {
				existing.CreatedAt = item.CreatedAt
			}
			assistant[key] = existing
			continue
		}
		appended, err := appendProjectedItem(ctx, store, cache, item)
		if err != nil {
			return err
		}
		if appended {
			result.Items++
		}
	}
	return nil
}

func appendProjectedItem(ctx context.Context, store *ItemStore, cache map[string]map[string]bool, item Item) (bool, error) {
	ids, err := cachedItemIDs(ctx, store, cache, item.SessionID)
	if err != nil {
		return false, err
	}
	if ids[item.ID] {
		return false, nil
	}
	saved, err := store.Append(ctx, item)
	if err != nil {
		return false, err
	}
	ids[saved.ID] = true
	return true, nil
}

func cachedTurns(ctx context.Context, store *TurnStore, cache map[string]map[string]Turn, sessionID string) (map[string]Turn, error) {
	if turns, ok := cache[sessionID]; ok {
		return turns, nil
	}
	list, err := store.ListBySession(ctx, sessionID)
	if err != nil {
		return nil, err
	}
	turns := map[string]Turn{}
	for _, turn := range list {
		turns[turn.ID] = turn
	}
	cache[sessionID] = turns
	return turns, nil
}

func cachedItemIDs(ctx context.Context, store *ItemStore, cache map[string]map[string]bool, sessionID string) (map[string]bool, error) {
	if ids, ok := cache[sessionID]; ok {
		return ids, nil
	}
	list, err := store.ListBySession(ctx, sessionID)
	if err != nil {
		return nil, err
	}
	ids := map[string]bool{}
	for _, item := range list {
		ids[item.ID] = true
	}
	cache[sessionID] = ids
	return ids, nil
}

func payloadItems(ev eventbus.Event) []Item {
	var payload struct {
		Items []Item `json:"items"`
	}
	if err := ev.DecodePayload(&payload); err != nil || len(payload.Items) == 0 {
		return nil
	}
	items := make([]Item, 0, len(payload.Items))
	for _, item := range payload.Items {
		if item.ID == "" {
			item.ID = ev.ID
		}
		if item.SessionID == "" {
			item.SessionID = ev.SessionID
		}
		if item.TurnID == "" {
			item.TurnID = ev.RunID
		}
		if item.CreatedAt.IsZero() {
			item.CreatedAt = ev.Time
		}
		items = append(items, item)
	}
	return items
}

func itemSingle(base Item, kind ItemKind, role string, text string) []Item {
	if base.SessionID == "" || (text == "" && len(base.RawJSON) == 0) {
		return nil
	}
	base.Kind = kind
	base.Role = role
	base.Text = text
	return []Item{base}
}

func titleFromEvent(ev eventbus.Event) string {
	if prompt := promptFromEvent(ev); prompt != "" {
		if len(prompt) > 80 {
			return strings.TrimSpace(prompt[:80])
		}
		return prompt
	}
	return "Session " + ev.SessionID
}

func promptFromEvent(ev eventbus.Event) string {
	var payload struct {
		Prompt string `json:"prompt"`
	}
	_ = ev.DecodePayload(&payload)
	return strings.TrimSpace(payload.Prompt)
}

func runErrorFromEvent(ev eventbus.Event) string {
	var payload struct {
		Error  string `json:"error"`
		Detail string `json:"detail"`
	}
	_ = ev.DecodePayload(&payload)
	if strings.TrimSpace(payload.Detail) != "" {
		return strings.TrimSpace(payload.Detail)
	}
	return strings.TrimSpace(payload.Error)
}

func eventTime(ev eventbus.Event) time.Time {
	if ev.Time.IsZero() {
		return time.Now().UTC()
	}
	return ev.Time
}

func rawObject(value map[string]any) json.RawMessage {
	data, _ := json.Marshal(value)
	return data
}

func firstNonEmpty(values ...string) string {
	for _, value := range values {
		if value != "" {
			return value
		}
	}
	return ""
}
