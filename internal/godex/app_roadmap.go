package godex

import (
	"context"

	"github.com/pandelisz/gode/internal/godex/roadmap"
)

func (a *App) ListRoadmaps(ctx context.Context) ([]roadmap.DocumentSummary, error) {
	return a.Roadmaps.ListDocuments(ctx)
}

func (a *App) OpenRoadmap(ctx context.Context, path string) (*roadmap.Document, error) {
	return a.Roadmaps.Open(ctx, path)
}

func (a *App) FocusRoadmapTask(ctx context.Context, path string, taskID string) error {
	return a.Roadmaps.FocusTask(ctx, path, taskID)
}

func (a *App) SetRoadmapTask(ctx context.Context, path string, taskID string, checked bool, evidence string) error {
	return a.Roadmaps.SetTask(ctx, path, taskID, checked, evidence)
}

func (a *App) ValidateRoadmap(ctx context.Context, path string) (roadmap.ValidationResult, error) {
	return a.Roadmaps.Validate(ctx, path)
}

func (a *App) RoadmapContextPrompt(ctx context.Context, path string) (string, error) {
	return a.Roadmaps.ContextPrompt(ctx, path)
}

func (a *App) ListRoadmapThreads(ctx context.Context, path string) ([]roadmap.ThreadAttachment, error) {
	return a.Roadmaps.ListThreads(ctx, path)
}

func (a *App) SpawnRoadmapThread(ctx context.Context, path string, taskID string) (roadmap.ThreadAttachment, error) {
	return a.Roadmaps.SpawnThread(ctx, path, taskID)
}

func (a *App) AttachRoadmapThread(ctx context.Context, path string, taskID string, threadID string) (roadmap.ThreadAttachment, error) {
	return a.Roadmaps.AttachThread(ctx, path, taskID, threadID)
}
