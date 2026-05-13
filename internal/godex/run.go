package godex

import (
	"context"

	"github.com/pandelisz/gode/internal/godex/agent"
)

func (a *App) Run(ctx context.Context, req agent.RunRequest) (agent.RunResult, error) {
	return a.runner.Run(ctx, req)
}

func (a *App) Steer(ctx context.Context, req agent.SteerRequest) (string, error) {
	return a.runner.Steer(ctx, req)
}
