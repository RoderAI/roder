package appserver

import (
	"context"
	"encoding/json"
	"sort"
	"time"

	"github.com/google/uuid"
)

type threadStartParams struct {
	CWD       string `json:"cwd"`
	Model     string `json:"model"`
	Provider  string `json:"modelProvider"`
	Ephemeral bool   `json:"ephemeral"`
}

func (c *Connection) handleThreadStart(ctx context.Context, raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[threadStartParams](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}

	now := time.Now().Unix()
	cwd := params.CWD
	if cwd == "" {
		cwd = c.server.app.Config.Workspace
	}
	model := params.Model
	if model == "" {
		model = c.server.app.Config.Model
	}
	provider := params.Provider
	if provider == "" {
		provider = c.server.app.Config.Provider
	}

	thread := Thread{
		ID:            uuid.NewString(),
		SessionID:     uuid.NewString(),
		Preview:       "",
		Ephemeral:     params.Ephemeral,
		ModelProvider: provider,
		CreatedAt:     now,
		UpdatedAt:     now,
		Status:        idleStatus(),
		CWD:           cwd,
		CLIVersion:    c.server.options.Version,
		Source:        "appServer",
		Turns:         []Turn{},
	}

	c.server.mu.Lock()
	c.server.threads[thread.ID] = &threadState{Thread: thread}
	c.server.mu.Unlock()
	c.subscribe(thread.ID)

	_ = c.sendNotification(ctx, "thread/started", map[string]any{"thread": thread})
	return map[string]any{
		"thread":        thread,
		"model":         model,
		"modelProvider": provider,
		"cwd":           cwd,
	}, nil
}

func (s *Server) handleThreadList(raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[struct {
		Limit uint32 `json:"limit"`
	}](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}

	s.mu.RLock()
	threads := make([]Thread, 0, len(s.threads))
	for _, state := range s.threads {
		thread := state.Thread
		thread.Turns = []Turn{}
		threads = append(threads, thread)
	}
	s.mu.RUnlock()

	sort.Slice(threads, func(i, j int) bool {
		return threads[i].UpdatedAt > threads[j].UpdatedAt
	})
	if params.Limit > 0 && len(threads) > int(params.Limit) {
		threads = threads[:params.Limit]
	}
	return map[string]any{"data": threads, "nextCursor": nil, "backwardsCursor": nil}, nil
}

func (s *Server) handleThreadLoadedList(raw json.RawMessage) (any, *RPCError) {
	if _, err := decodeParams[map[string]any](raw); err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	s.mu.RLock()
	ids := make([]string, 0, len(s.threads))
	for id := range s.threads {
		ids = append(ids, id)
	}
	s.mu.RUnlock()
	sort.Strings(ids)
	return map[string]any{"data": ids, "nextCursor": nil}, nil
}

func (s *Server) handleThreadRead(raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[struct {
		ThreadID     string `json:"threadId"`
		IncludeTurns bool   `json:"includeTurns"`
	}](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	if params.ThreadID == "" {
		return nil, rpcError(errorInvalidParams, "threadId is required")
	}
	s.mu.RLock()
	state := s.threads[params.ThreadID]
	s.mu.RUnlock()
	if state == nil {
		return nil, rpcError(errorInvalidParams, "thread not found")
	}
	thread := state.Thread
	if !params.IncludeTurns {
		thread.Turns = []Turn{}
	}
	return map[string]any{"thread": thread}, nil
}

func (c *Connection) handleThreadUnsubscribe(raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[struct {
		ThreadID string `json:"threadId"`
	}](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	c.server.mu.RLock()
	_, loaded := c.server.threads[params.ThreadID]
	c.server.mu.RUnlock()
	if !loaded {
		return map[string]any{"status": "notLoaded"}, nil
	}
	if !c.unsubscribe(params.ThreadID) {
		return map[string]any{"status": "notSubscribed"}, nil
	}
	return map[string]any{"status": "unsubscribed"}, nil
}
