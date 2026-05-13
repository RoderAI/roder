package godex

import (
	"context"

	"github.com/pandelisz/gode/internal/godex/goals"
)

func (a *App) GetGoal(ctx context.Context, sessionID string) (*goals.Goal, error) {
	if a.Goals == nil {
		return nil, nil
	}
	return a.Goals.Get(ctx, sessionID)
}

func (a *App) SetGoal(ctx context.Context, req goals.SetRequest) (*goals.Goal, error) {
	if a.Goals == nil {
		return nil, goals.ErrNotFound
	}
	return a.Goals.Set(ctx, req)
}

func (a *App) ClearGoal(ctx context.Context, sessionID string) error {
	if a.Goals == nil {
		return nil
	}
	return a.Goals.Clear(ctx, sessionID)
}
