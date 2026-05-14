package agent

import (
	"context"
	"strings"
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
	InputItems   []provider.Item
	Final        string
	ResponseID   string
	HadToolCall  bool
	ProducedText bool
	Usage        provider.TokenUsage
}

type pendingToolCall struct {
	request *provider.ToolRequest
	items   []provider.Item
}

func (r *Runner) streamProviderTurn(ctx context.Context, req RunRequest, providerReq provider.Request, messages []provider.Message, inputItems []provider.Item, final string, stats *runStats, allowTools bool) (turnOutcome, error) {
	events, errs := r.provider.Stream(ctx, providerReq)
	outcome := turnOutcome{
		Messages:   messages,
		InputItems: append([]provider.Item(nil), inputItems...),
		Final:      final,
	}
	storedItemKeys := map[string]bool{}
	var pendingTools []pendingToolCall
	persistItemsOnce := func(items []provider.Item, final string) error {
		persistCtx := ctx
		if ctx.Err() != nil {
			persistCtx = context.WithoutCancel(ctx)
		}
		filtered := make([]provider.Item, 0, len(items))
		for _, item := range items {
			key := providerItemStorageKey(item)
			if key != "" {
				if storedItemKeys[key] {
					continue
				}
				storedItemKeys[key] = true
			}
			filtered = append(filtered, item)
		}
		return r.persistProviderItems(persistCtx, req, filtered, final)
	}
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
					items := ev.Items
					if len(items) == 0 {
						items = []provider.Item{providerItemFromToolRequest(ev.ToolRequest)}
					}
					if err := persistItemsOnce(items, ""); err != nil {
						return outcome, err
					}
					pendingTools = append(pendingTools, pendingToolCall{request: ev.ToolRequest, items: items})
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
				if err := persistItemsOnce(ev.Items, outcome.Final); err != nil {
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
			if len(pendingTools) > 0 {
				nextMessages, nextInputItems, err := r.cancelPendingToolCalls(ctx, req, outcome.Messages, outcome.InputItems, pendingTools, persistItemsOnce, ctx.Err())
				if err != nil {
					return outcome, err
				}
				outcome.Messages = nextMessages
				outcome.InputItems = nextInputItems
			}
			return outcome, ctx.Err()
		}
	}
	if len(pendingTools) > 0 {
		nextMessages, nextInputItems, err := r.runToolCalls(ctx, req, outcome.Messages, outcome.InputItems, pendingTools, persistItemsOnce)
		if err != nil {
			return outcome, err
		}
		outcome.Messages = nextMessages
		outcome.InputItems = nextInputItems
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

func (r *Runner) runToolCalls(ctx context.Context, req RunRequest, messages []provider.Message, inputItems []provider.Item, toolRequests []pendingToolCall, persistItems func([]provider.Item, string) error) ([]provider.Message, []provider.Item, error) {
	inputItems = append([]provider.Item(nil), inputItems...)
	for _, toolCall := range toolRequests {
		toolRequest := toolCall.request
		messages = append(messages, provider.Message{
			Role:          provider.RoleAssistant,
			ToolCallID:    toolRequest.ID,
			ToolName:      toolRequest.Name,
			ToolArguments: toolRequest.Arguments,
		})
		if len(toolCall.items) > 0 {
			inputItems = append(inputItems, toolCall.items...)
		} else {
			inputItems = append(inputItems, providerItemFromToolRequest(toolRequest))
		}
	}

	results := make([]tools.Result, len(toolRequests))
	errs := make([]error, len(toolRequests))
	var wg sync.WaitGroup
	for i, toolCall := range toolRequests {
		toolRequest := toolCall.request
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
	for i, err := range errs {
		if err != nil {
			results[i] = failedToolCallResult(results[i], err)
			toolRequest := toolRequests[i].request
			r.emit(context.WithoutCancel(ctx), eventbus.Event{
				Kind:      eventbus.KindToolFailed,
				Source:    eventbus.SourceTool,
				SessionID: req.SessionID,
				RunID:     req.RunID,
				Payload: map[string]any{
					"tool_call_id": toolRequest.ID,
					"tool":         toolRequest.Name,
					"error":        results[i].Error,
					"text":         results[i].Text,
				},
			})
		}
	}
	var outputItems []provider.Item
	for i, toolCall := range toolRequests {
		toolRequest := toolCall.request
		content := toolResponseContent(toolRequest.Name, results[i])
		messages = append(messages, provider.Message{
			Role:       provider.RoleTool,
			ToolCallID: toolRequest.ID,
			Content:    content,
		})
		outputItems = append(outputItems, provider.Item{
			Kind:       provider.ItemFunctionOut,
			Role:       string(provider.RoleTool),
			ToolName:   toolRequest.Name,
			ToolCallID: toolRequest.ID,
			Text:       content,
		})
	}
	implicitMessages, implicitItems, err := r.implicitSkillMessages(ctx, toolRequests)
	if err != nil {
		return messages, inputItems, err
	}
	if len(implicitMessages) > 0 {
		messages = append(messages, implicitMessages...)
		outputItems = append(outputItems, implicitItems...)
	}
	if len(outputItems) > 0 {
		if err := persistItems(outputItems, ""); err != nil {
			return messages, inputItems, err
		}
		inputItems = append(inputItems, outputItems...)
	}
	return messages, inputItems, nil
}

func (r *Runner) cancelPendingToolCalls(ctx context.Context, req RunRequest, messages []provider.Message, inputItems []provider.Item, toolRequests []pendingToolCall, persistItems func([]provider.Item, string) error, cause error) ([]provider.Message, []provider.Item, error) {
	inputItems = append([]provider.Item(nil), inputItems...)
	outputItems := make([]provider.Item, 0, len(toolRequests))
	for _, toolCall := range toolRequests {
		toolRequest := toolCall.request
		messages = append(messages, provider.Message{
			Role:          provider.RoleAssistant,
			ToolCallID:    toolRequest.ID,
			ToolName:      toolRequest.Name,
			ToolArguments: toolRequest.Arguments,
		})
		if len(toolCall.items) > 0 {
			inputItems = append(inputItems, toolCall.items...)
		} else {
			inputItems = append(inputItems, providerItemFromToolRequest(toolRequest))
		}
		result := failedToolCallResult(tools.Result{}, cause)
		content := toolResponseContent(toolRequest.Name, result)
		messages = append(messages, provider.Message{
			Role:       provider.RoleTool,
			ToolCallID: toolRequest.ID,
			Content:    content,
		})
		outputItems = append(outputItems, provider.Item{
			Kind:       provider.ItemFunctionOut,
			Role:       string(provider.RoleTool),
			ToolName:   toolRequest.Name,
			ToolCallID: toolRequest.ID,
			Text:       content,
		})
		r.emit(context.WithoutCancel(ctx), eventbus.Event{
			Kind:      eventbus.KindToolFailed,
			Source:    eventbus.SourceTool,
			SessionID: req.SessionID,
			RunID:     req.RunID,
			Payload: map[string]any{
				"tool_call_id": toolRequest.ID,
				"tool":         toolRequest.Name,
				"error":        result.Error,
				"text":         result.Text,
			},
		})
	}
	if len(outputItems) > 0 {
		if err := persistItems(outputItems, ""); err != nil {
			return messages, inputItems, err
		}
		inputItems = append(inputItems, outputItems...)
	}
	return messages, inputItems, nil
}

func failedToolCallResult(result tools.Result, err error) tools.Result {
	message := strings.TrimSpace(err.Error())
	output := strings.TrimSpace(result.Text)
	result.Error = message
	switch {
	case message == "":
		result.Text = output
	case output == "":
		result.Text = message
	case strings.Contains(message, output):
		result.Text = message
	default:
		result.Text = message + "\n" + output
	}
	return result
}

func providerItemFromToolRequest(toolRequest *provider.ToolRequest) provider.Item {
	if toolRequest == nil {
		return provider.Item{}
	}
	return provider.Item{
		Kind:       provider.ItemFunctionCall,
		ToolName:   toolRequest.Name,
		ToolCallID: toolRequest.ID,
		Text:       toolRequest.Arguments,
	}
}

func providerItemStorageKey(item provider.Item) string {
	if item.ID != "" {
		return string(item.Kind) + ":id:" + item.ID
	}
	if item.ToolCallID != "" {
		return string(item.Kind) + ":tool:" + item.ToolCallID
	}
	if item.Kind == provider.ItemMessage && item.Role != "" && item.Text != "" {
		return string(item.Kind) + ":message:" + item.Role + ":" + item.Text
	}
	return ""
}
