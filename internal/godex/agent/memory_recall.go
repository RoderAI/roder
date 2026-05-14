package agent

import (
	"context"

	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/provider"
)

func (r *Runner) memoryRecallMessage(ctx context.Context, req RunRequest, prompt string) (provider.Message, bool) {
	if r.memory == nil {
		return provider.Message{}, false
	}
	result, err := r.memory.Recall(ctx, prompt)
	if err != nil {
		r.emit(ctx, eventbus.Event{
			Kind:      eventbus.KindMemoryRecallFailed,
			Source:    eventbus.SourceAgent,
			SessionID: req.SessionID,
			RunID:     req.RunID,
			Payload: map[string]any{
				"source": "prompt",
				"query":  result.Query,
				"model":  result.Model,
				"error":  err.Error(),
			},
		})
		return provider.Message{}, false
	}
	if result.Query == "" {
		return provider.Message{}, false
	}
	r.emit(ctx, eventbus.Event{
		Kind:      eventbus.KindMemoryRecalled,
		Source:    eventbus.SourceAgent,
		SessionID: req.SessionID,
		RunID:     req.RunID,
		Payload: map[string]any{
			"source":     "prompt",
			"query":      result.Query,
			"count":      len(result.Entries),
			"model":      result.Model,
			"memory_ids": result.MemoryIDs,
		},
	})
	if result.Text == "" {
		return provider.Message{}, false
	}
	return provider.Message{Role: provider.RoleUser, Content: result.Text}, true
}
