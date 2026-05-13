package agent

import (
	"context"
	"strings"
	"time"

	"github.com/pandelisz/gode/internal/godex/contextwindow"
	"github.com/pandelisz/gode/internal/godex/goals"
	"github.com/pandelisz/gode/internal/godex/provider"
)

func (r *Runner) goalContextMessage(ctx context.Context, sessionID string) (provider.Message, bool, error) {
	if r.goals == nil || strings.TrimSpace(sessionID) == "" {
		return provider.Message{}, false, nil
	}
	goal, err := r.goals.Get(ctx, sessionID)
	if err != nil || goal == nil || !goals.IsContinuable(goal.Status) {
		return provider.Message{}, false, err
	}
	remaining := int64(-1)
	if value := goals.RemainingTokens(*goal); value != nil {
		remaining = *value
	}
	return provider.Message{Role: provider.RoleSystem, Content: goals.ContinuationPrompt(*goal, remaining)}, true, nil
}

func (r *Runner) recordGoalUsage(ctx context.Context, req RunRequest, messages []provider.Message, elapsed time.Duration) {
	if r.goals == nil {
		return
	}
	model := firstNonEmpty(r.model, "gpt-5.5")
	window := contextwindow.ForModel(model)
	estimate := contextwindow.EstimateMessages(contextWindowMessages(messages), window)
	_, _ = r.goals.AddUsage(ctx, req.SessionID, int64(estimate.Tokens), elapsed)
}
