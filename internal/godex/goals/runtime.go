package goals

import (
	"context"
	"time"

	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/journal"
)

type Runtime struct {
	Store   *Store
	Bus     *eventbus.Bus
	Journal *journal.Store
}

func NewRuntime(store *Store, bus *eventbus.Bus, journal *journal.Store) *Runtime {
	return &Runtime{Store: store, Bus: bus, Journal: journal}
}

func (r *Runtime) Get(ctx context.Context, sessionID string) (*Goal, error) {
	if r == nil || r.Store == nil {
		return nil, nil
	}
	return r.Store.Get(ctx, sessionID)
}

func (r *Runtime) Set(ctx context.Context, req SetRequest) (*Goal, error) {
	if r == nil || r.Store == nil {
		return nil, ErrNotFound
	}
	goal, err := r.Store.Set(ctx, req)
	if err != nil {
		return nil, err
	}
	r.publish(ctx, eventbus.KindGoalUpdated, *goal)
	return goal, nil
}

func (r *Runtime) Clear(ctx context.Context, sessionID string) error {
	if r == nil || r.Store == nil {
		return nil
	}
	if err := r.Store.Clear(ctx, sessionID); err != nil {
		return err
	}
	r.publish(ctx, eventbus.KindGoalCleared, Goal{SessionID: sessionID})
	return nil
}

func (r *Runtime) AddUsage(ctx context.Context, sessionID string, tokens int64, elapsed time.Duration) (*Goal, error) {
	if r == nil || r.Store == nil {
		return nil, nil
	}
	goal, err := r.Store.AddUsage(ctx, sessionID, tokens, elapsed)
	if err != nil || goal == nil {
		return goal, err
	}
	kind := eventbus.KindGoalUpdated
	if goal.Status == StatusBudgetLimited {
		kind = eventbus.KindGoalLimited
	}
	r.publish(ctx, kind, *goal)
	return goal, nil
}

func (r *Runtime) publish(ctx context.Context, kind eventbus.Kind, goal Goal) {
	if r == nil {
		return
	}
	payload := Payload(goal)
	ev := eventbus.Event{
		Kind:      kind,
		Source:    eventbus.SourceAgent,
		SessionID: goal.SessionID,
		Payload:   payload,
	}
	if r.Bus != nil {
		ev = r.Bus.Publish(ctx, ev)
	}
	if r.Journal != nil {
		_ = r.Journal.Append(ctx, ev)
	}
}

func Payload(goal Goal) map[string]any {
	payload := map[string]any{
		"session_id":         goal.SessionID,
		"goal_id":            goal.GoalID,
		"status":             string(goal.Status),
		"objective":          goal.Objective,
		"tokens_used":        goal.TokensUsed,
		"time_used_seconds":  goal.TimeUsedSeconds,
		"created_at":         goal.CreatedAt,
		"updated_at":         goal.UpdatedAt,
		"remaining_tokens":   RemainingTokens(goal),
		"completion_summary": CompletionBudgetReport(goal),
	}
	if goal.TokenBudget != nil {
		payload["token_budget"] = *goal.TokenBudget
	}
	return payload
}

func RemainingTokens(goal Goal) *int64 {
	if goal.TokenBudget == nil {
		return nil
	}
	remaining := *goal.TokenBudget - goal.TokensUsed
	if remaining < 0 {
		remaining = 0
	}
	return &remaining
}

func CompletionBudgetReport(goal Goal) string {
	if goal.TokenBudget == nil {
		return "No token budget was set for this goal."
	}
	remaining := int64(0)
	if goal.TokenBudget != nil {
		remaining = *goal.TokenBudget - goal.TokensUsed
		if remaining < 0 {
			remaining = 0
		}
	}
	return "Goal used " + formatInt(goal.TokensUsed) + " of " + formatInt(*goal.TokenBudget) + " tokens; " + formatInt(remaining) + " remain."
}

func formatInt(value int64) string {
	if value == 0 {
		return "0"
	}
	var digits [20]byte
	i := len(digits)
	for value > 0 {
		i--
		digits[i] = byte('0' + value%10)
		value /= 10
	}
	return string(digits[i:])
}
