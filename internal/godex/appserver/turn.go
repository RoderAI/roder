package appserver

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"time"

	"github.com/google/uuid"
	"github.com/pandelisz/gode/internal/godex/agent"
	"github.com/pandelisz/gode/internal/godex/eventbus"
)

type turnStartParams struct {
	ThreadID string            `json:"threadId"`
	Input    []json.RawMessage `json:"input"`
	Prompt   string            `json:"prompt"`
}

func (c *Connection) handleTurnStart(ctx context.Context, raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[turnStartParams](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	if params.ThreadID == "" {
		return nil, rpcError(errorInvalidParams, "threadId is required")
	}
	prompt, err := params.prompt()
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	if strings.TrimSpace(prompt) == "" {
		return nil, rpcError(errorInvalidParams, "input text is required")
	}

	now := time.Now().Unix()
	turn := Turn{
		ID:        uuid.NewString(),
		Items:     []any{},
		ItemsView: "full",
		Status:    "inProgress",
		StartedAt: &now,
	}

	storedThread, storedFound, rpcErr := c.server.readStoredThread(context.Background(), params.ThreadID, false)
	if rpcErr != nil {
		return nil, rpcErr
	}
	c.server.mu.Lock()
	state := c.server.threads[params.ThreadID]
	if state == nil {
		if !storedFound {
			c.server.mu.Unlock()
			return nil, rpcError(errorInvalidParams, "thread not found")
		}
		state = &threadState{Thread: storedThread}
		c.server.threads[params.ThreadID] = state
	}
	state.Status = activeStatus()
	state.UpdatedAt = now
	state.Turns = append(state.Turns, turn)
	runCtx, cancel := context.WithCancel(context.Background())
	state.activeCancel = cancel
	c.server.mu.Unlock()

	c.subscribe(params.ThreadID)
	go c.server.runTurn(runCtx, params.ThreadID, turn.ID, prompt)
	return map[string]any{"turn": turn}, nil
}

func (s *Server) handleTurnInterrupt(ctx context.Context, raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[struct {
		ThreadID string `json:"threadId"`
		TurnID   string `json:"turnId"`
	}](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	s.mu.Lock()
	state := s.threads[params.ThreadID]
	if state == nil {
		s.mu.Unlock()
		return nil, rpcError(errorInvalidParams, "thread not found")
	}
	cancel := state.activeCancel
	s.mu.Unlock()
	if cancel == nil {
		return map[string]any{}, nil
	}
	cancel()
	s.app.Bus.Publish(ctx, eventbus.Event{
		Kind:      eventbus.KindRunCancelRequested,
		Source:    eventbus.SourceSystem,
		SessionID: params.ThreadID,
		RunID:     params.TurnID,
	})
	return map[string]any{}, nil
}

func (s *Server) runTurn(ctx context.Context, threadID, turnID, prompt string) {
	start := time.Now()
	itemID := uuid.NewString()
	subCtx, cancelSub := context.WithCancel(context.Background())
	events := s.app.Bus.Subscribe(subCtx, eventbus.Filter{SessionID: threadID, RunID: turnID})
	defer cancelSub()

	s.notifyThread(ctx, threadID, "thread/status/changed", map[string]any{
		"threadId": threadID,
		"status":   activeStatus(),
	})
	s.notifyThread(ctx, threadID, "turn/started", map[string]any{
		"threadId": threadID,
		"turn":     s.turnSnapshot(threadID, turnID),
	})
	s.notifyThread(ctx, threadID, "item/started", map[string]any{
		"threadId":    threadID,
		"turnId":      turnID,
		"startedAtMs": time.Now().UnixMilli(),
		"item": map[string]any{
			"id":   itemID,
			"type": "agentMessage",
			"text": "",
		},
	})

	resultCh := make(chan agent.RunResult, 1)
	errCh := make(chan error, 1)
	go func() {
		result, err := s.app.Run(ctx, agent.RunRequest{SessionID: threadID, RunID: turnID, Prompt: prompt, Resume: true})
		if err != nil {
			errCh <- err
			return
		}
		resultCh <- result
	}()

	finalText := ""
	var runResult agent.RunResult
	var runErr error
	done := false
	for !done {
		select {
		case ev := <-events:
			finalText += s.handleRunEvent(ctx, threadID, turnID, itemID, ev)
		case runResult = <-resultCh:
			done = true
		case runErr = <-errCh:
			done = true
		}
	}
drain:
	for {
		select {
		case ev := <-events:
			finalText += s.handleRunEvent(ctx, threadID, turnID, itemID, ev)
		default:
			break drain
		}
	}
	if runResult.FinalText != "" {
		finalText = runResult.FinalText
	}

	status := "completed"
	var turnErr *TurnError
	if runErr != nil {
		if errors.Is(runErr, context.Canceled) {
			status = "interrupted"
		} else {
			status = "failed"
			turnErr = &TurnError{Message: runErr.Error()}
		}
	}
	s.completeTurn(threadID, turnID, itemID, finalText, status, turnErr, start)
}

func (s *Server) handleRunEvent(ctx context.Context, threadID, turnID, itemID string, ev eventbus.Event) string {
	switch ev.Kind {
	case eventbus.KindAssistantDelta:
		var payload struct {
			Text string `json:"text"`
		}
		_ = ev.DecodePayload(&payload)
		if payload.Text != "" {
			s.notifyThread(ctx, threadID, "item/agentMessage/delta", map[string]any{
				"threadId": threadID,
				"turnId":   turnID,
				"itemId":   itemID,
				"delta":    payload.Text,
			})
		}
		return payload.Text
	case eventbus.KindToolRequested, eventbus.KindToolStarted, eventbus.KindToolCompleted, eventbus.KindToolFailed:
		s.notifyThread(ctx, threadID, "item/completed", map[string]any{
			"threadId":      threadID,
			"turnId":        turnID,
			"completedAtMs": time.Now().UnixMilli(),
			"item": map[string]any{
				"id":      ev.ID,
				"type":    string(ev.Kind),
				"payload": ev.Payload,
			},
		})
	}
	return ""
}

func (s *Server) completeTurn(threadID, turnID, itemID, finalText, status string, turnErr *TurnError, start time.Time) {
	completed := time.Now().Unix()
	duration := time.Since(start).Milliseconds()
	item := map[string]any{"id": itemID, "type": "agentMessage", "text": finalText}

	s.mu.Lock()
	state := s.threads[threadID]
	var completedTurn Turn
	if state != nil {
		state.Status = idleStatus()
		state.UpdatedAt = completed
		state.activeCancel = nil
		for i := range state.Turns {
			if state.Turns[i].ID == turnID {
				state.Turns[i].Status = status
				state.Turns[i].Error = turnErr
				state.Turns[i].CompletedAt = &completed
				state.Turns[i].DurationMs = &duration
				state.Turns[i].Items = append(state.Turns[i].Items, item)
				completedTurn = state.Turns[i]
				break
			}
		}
	}
	s.mu.Unlock()

	ctx := context.Background()
	s.notifyThread(ctx, threadID, "item/completed", map[string]any{
		"threadId":      threadID,
		"turnId":        turnID,
		"completedAtMs": time.Now().UnixMilli(),
		"item":          item,
	})
	s.notifyThread(ctx, threadID, "turn/completed", map[string]any{
		"threadId": threadID,
		"turn":     completedTurn,
	})
	s.notifyThread(ctx, threadID, "thread/status/changed", map[string]any{
		"threadId": threadID,
		"status":   idleStatus(),
	})
}

func (s *Server) turnSnapshot(threadID, turnID string) Turn {
	s.mu.RLock()
	defer s.mu.RUnlock()
	state := s.threads[threadID]
	if state == nil {
		return Turn{}
	}
	for _, turn := range state.Turns {
		if turn.ID == turnID {
			return turn
		}
	}
	return Turn{}
}

func (p turnStartParams) prompt() (string, error) {
	if p.Prompt != "" {
		return p.Prompt, nil
	}
	var parts []string
	for _, raw := range p.Input {
		var item struct {
			Type string `json:"type"`
			Text string `json:"text"`
		}
		if err := json.Unmarshal(raw, &item); err != nil {
			return "", err
		}
		if item.Text != "" {
			parts = append(parts, item.Text)
		}
	}
	return strings.Join(parts, "\n"), nil
}
