package agent

import (
	"context"
	"strings"
	"time"

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

func (r *Runner) recordGoalUsage(ctx context.Context, req RunRequest, usage provider.TokenUsage, elapsed time.Duration) {
	if r.goals == nil || usage.IsZero() {
		return
	}
	_, _ = r.goals.AddUsage(ctx, req.SessionID, usage.Total(), elapsed)
}
