package agent

import (
	"context"
	"sync"

	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/provider"
	"github.com/pandelisz/gode/internal/godex/tools"
)

type runStats struct {
	ToolTurns      int
	ToolCalls      int
	LastTool       string
	LastToolCallID string
	LastResponseID string
	TokenUsage     provider.TokenUsage
}

type turnOutcome struct {
	Messages     []provider.Message
	Final        string
	ResponseID   string
	HadToolCall  bool
	ProducedText bool
	Usage        provider.TokenUsage
}

func (r *Runner) streamProviderTurn(ctx context.Context, req RunRequest, providerReq provider.Request, messages []provider.Message, final string, stats *runStats, allowTools bool) (turnOutcome, error) {
	events, errs := r.provider.Stream(ctx, providerReq)
	outcome := turnOutcome{Messages: messages, Final: final}
	var pendingTools []*provider.ToolRequest
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
					Payload:   map[string]any{"text": ev.Text, "phase": ev.Phase},
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
					pendingTools = append(pendingTools, ev.ToolRequest)
				}
			case provider.EventCompleted:
				outcome.ResponseID = ev.ResponseID
				if ev.ResponseID != "" {
					stats.LastResponseID = ev.ResponseID
				}
				outcome.Usage = ev.Usage
				stats.TokenUsage = addTokenUsage(stats.TokenUsage, ev.Usage)
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
					Payload: map[string]any{
						"text":        outcome.Final,
						"phase":       provider.PhaseFinalAnswer,
						"response_id": ev.ResponseID,
						"items":       r.sessionItemsFromProviderItems(req, ev.Items),
						"usage":       ev.Usage,
					},
				})
				r.emitActualTokenUsage(ctx, req, ev.Usage)
				r.addSessionTokenUsage(ctx, req, ev.Usage)
				if err := r.persistProviderItems(ctx, req, ev.Items, outcome.Final); err != nil {
					return outcome, err
				}
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
	if len(pendingTools) > 0 {
		nextMessages, err := r.runToolCalls(ctx, req, outcome.Messages, pendingTools)
		if err != nil {
			return outcome, err
		}
		outcome.Messages = nextMessages
	}
	return outcome, nil
}

func addTokenUsage(a provider.TokenUsage, b provider.TokenUsage) provider.TokenUsage {
	return provider.TokenUsage{
		InputTokens:  a.InputTokens + b.InputTokens,
		OutputTokens: a.OutputTokens + b.OutputTokens,
		TotalTokens:  a.Total() + b.Total(),
	}
}

func (r *Runner) addSessionTokenUsage(ctx context.Context, req RunRequest, usage provider.TokenUsage) {
	if r.sessions == nil || usage.IsZero() {
		return
	}
	_, _ = r.sessions.AddTokenUsage(ctx, req.SessionID, usage.InputTokens, usage.OutputTokens)
}

func (r *Runner) runToolCalls(ctx context.Context, req RunRequest, messages []provider.Message, toolRequests []*provider.ToolRequest) ([]provider.Message, error) {
	for _, toolRequest := range toolRequests {
		messages = append(messages, provider.Message{
			Role:          provider.RoleAssistant,
			ToolCallID:    toolRequest.ID,
			ToolName:      toolRequest.Name,
			ToolArguments: toolRequest.Arguments,
		})
	}

	results := make([]tools.Result, len(toolRequests))
	errs := make([]error, len(toolRequests))
	var wg sync.WaitGroup
	for i, toolRequest := range toolRequests {
		wg.Add(1)
		go func(i int, toolRequest *provider.ToolRequest) {
			defer wg.Done()
			results[i], errs[i] = r.tools.Run(ctx, tools.Call{
				ID:        toolRequest.ID,
				Name:      toolRequest.Name,
				Input:     toolRequest.Input,
				SessionID: req.SessionID,
				RunID:     req.RunID,
			})
		}(i, toolRequest)
	}
	wg.Wait()
	for _, err := range errs {
		if err != nil {
			return messages, err
		}
	}
	for i, toolRequest := range toolRequests {
		messages = append(messages, provider.Message{
			Role:       provider.RoleTool,
			ToolCallID: toolRequest.ID,
			Content:    toolResponseContent(toolRequest.Name, results[i]),
		})
	}
	return messages, nil
}
