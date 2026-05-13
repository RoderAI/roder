package agent

import (
	"context"

	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/provider"
	"github.com/pandelisz/gode/internal/godex/tools"
)

type runStats struct {
	ToolTurns      int
	ToolCalls      int
	LastTool       string
	LastToolCallID string
}

type turnOutcome struct {
	Messages     []provider.Message
	Final        string
	HadToolCall  bool
	ProducedText bool
}

func (r *Runner) streamProviderTurn(ctx context.Context, req RunRequest, providerReq provider.Request, messages []provider.Message, final string, stats *runStats, allowTools bool) (turnOutcome, error) {
	events, errs := r.provider.Stream(ctx, providerReq)
	outcome := turnOutcome{Messages: messages, Final: final}
	for events != nil || errs != nil {
		select {
		case ev, ok := <-events:
			if !ok {
				events = nil
				continue
			}
			switch ev.Kind {
			case provider.EventDelta:
				outcome.ProducedText = true
				outcome.Final += ev.Text
				r.emit(ctx, eventbus.Event{
					Kind:      eventbus.KindAssistantDelta,
					Source:    eventbus.SourceProvider,
					SessionID: req.SessionID,
					RunID:     req.RunID,
					Payload:   map[string]any{"text": ev.Text},
				})
			case provider.EventReasoningSummaryDelta:
				r.emit(ctx, eventbus.Event{
					Kind:      eventbus.KindReasoningSummaryDelta,
					Source:    eventbus.SourceProvider,
					SessionID: req.SessionID,
					RunID:     req.RunID,
					Payload:   map[string]any{"text": ev.Text},
				})
			case provider.EventReasoningSummaryDone:
				r.emit(ctx, eventbus.Event{
					Kind:      eventbus.KindReasoningSummaryCompleted,
					Source:    eventbus.SourceProvider,
					SessionID: req.SessionID,
					RunID:     req.RunID,
					Payload:   map[string]any{"text": ev.Text},
				})
			case provider.EventToolCall:
				outcome.HadToolCall = true
				if ev.ToolRequest == nil {
					continue
				}
				stats.ToolCalls++
				stats.LastTool = ev.ToolRequest.Name
				stats.LastToolCallID = ev.ToolRequest.ID
				r.emit(ctx, eventbus.Event{
					Kind:      eventbus.KindToolRequested,
					Source:    eventbus.SourceProvider,
					SessionID: req.SessionID,
					RunID:     req.RunID,
					Payload: map[string]any{
						"tool_call_id": ev.ToolRequest.ID,
						"tool":         ev.ToolRequest.Name,
						"input":        ev.ToolRequest.Input,
					},
				})
				if allowTools && r.tools != nil {
					var err error
					outcome.Messages, err = r.runToolCall(ctx, req, outcome.Messages, ev.ToolRequest)
					if err != nil {
						return outcome, err
					}
				}
			case provider.EventCompleted:
				if outcome.Final == "" {
					outcome.Final = ev.Text
				}
				if ev.Text != "" || outcome.Final != "" {
					outcome.ProducedText = true
				}
				r.emit(ctx, eventbus.Event{
					Kind:      eventbus.KindAssistantCompleted,
					Source:    eventbus.SourceProvider,
					SessionID: req.SessionID,
					RunID:     req.RunID,
					Payload:   map[string]any{"text": outcome.Final},
				})
			}
		case err, ok := <-errs:
			if !ok {
				errs = nil
				continue
			}
			if err != nil {
				return outcome, err
			}
		case <-ctx.Done():
			return outcome, ctx.Err()
		}
	}
	return outcome, nil
}

func (r *Runner) runToolCall(ctx context.Context, req RunRequest, messages []provider.Message, toolRequest *provider.ToolRequest) ([]provider.Message, error) {
	messages = append(messages, provider.Message{
		Role:          provider.RoleAssistant,
		ToolCallID:    toolRequest.ID,
		ToolName:      toolRequest.Name,
		ToolArguments: toolRequest.Arguments,
	})
	result, err := r.tools.Run(ctx, tools.Call{
		ID:        toolRequest.ID,
		Name:      toolRequest.Name,
		Input:     toolRequest.Input,
		SessionID: req.SessionID,
		RunID:     req.RunID,
	})
	if err != nil {
		return messages, err
	}
	messages = append(messages, provider.Message{
		Role:       provider.RoleTool,
		ToolCallID: toolRequest.ID,
		Content:    toolResponseContent(toolRequest.Name, result),
	})
	return messages, nil
}
