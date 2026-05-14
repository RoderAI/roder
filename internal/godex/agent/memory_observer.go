package agent

import (
	"context"
	"time"

	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/memory"
	"github.com/pandelisz/gode/internal/godex/provider"
)

const memoryObserverToolThreshold = 15

func (r *Runner) maybeStartMemoryObserver(req RunRequest, active *activeRun, messages []provider.Message, toolCalls int) {
	if r.memoryObserver == nil || !r.memoryObserver.Enabled() || toolCalls < memoryObserverToolThreshold {
		return
	}
	if !r.markMemoryObserverStarted(active) {
		return
	}
	snapshot := append([]provider.Message(nil), messages...)
	payload := map[string]any{"tool_calls": toolCalls, "messages": len(snapshot)}
	r.emit(context.Background(), eventbus.Event{
		Kind:      eventbus.KindMemoryObserverStarted,
		Source:    eventbus.SourceAgent,
		SessionID: req.SessionID,
		RunID:     req.RunID,
		Payload:   payload,
	})
	go r.runMemoryObserver(req, snapshot, toolCalls)
}

func (r *Runner) runMemoryObserver(req RunRequest, messages []provider.Message, toolCalls int) {
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	err := r.memoryObserver.Observe(ctx, memory.ObservationRequest{
		SessionID: req.SessionID,
		RunID:     req.RunID,
		Messages:  messages,
	})
	if err != nil {
		r.emit(context.Background(), eventbus.Event{
			Kind:      eventbus.KindMemoryObserverFailed,
			Source:    eventbus.SourceAgent,
			SessionID: req.SessionID,
			RunID:     req.RunID,
			Payload: map[string]any{
				"error":      err.Error(),
				"tool_calls": toolCalls,
			},
		})
		return
	}
	r.emit(context.Background(), eventbus.Event{
		Kind:      eventbus.KindMemoryObserverCompleted,
		Source:    eventbus.SourceAgent,
		SessionID: req.SessionID,
		RunID:     req.RunID,
		Payload:   map[string]any{"tool_calls": toolCalls},
	})
}
