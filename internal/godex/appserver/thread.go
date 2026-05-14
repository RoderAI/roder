package appserver

import (
	"context"
	"encoding/json"
	"sort"
	"time"

	"github.com/google/uuid"
	messagestore "github.com/pandelisz/gode/internal/godex/message"
	"github.com/pandelisz/gode/internal/godex/session"
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
	threadID := uuid.NewString()
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
		ID:            threadID,
		SessionID:     threadID,
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
	if c.server.app.Sessions != nil {
		if _, err := c.server.app.Sessions.Ensure(ctx, session.Session{
			ID:        thread.ID,
			Workspace: cwd,
			Model:     model,
			Provider:  provider,
			CreatedAt: time.Unix(now, 0).UTC(),
			UpdatedAt: time.Unix(now, 0).UTC(),
		}); err != nil {
			return nil, rpcError(errorInternal, err.Error())
		}
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

	threads, rpcErr := s.storedThreads(context.Background(), false)
	if rpcErr != nil {
		return nil, rpcErr
	}
	if threads == nil {
		s.mu.RLock()
		threads = make([]Thread, 0, len(s.threads))
		for _, state := range s.threads {
			thread := state.Thread
			thread.Turns = []Turn{}
			threads = append(threads, thread)
		}
		s.mu.RUnlock()
	}

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
	var ids []string
	if s.app != nil && s.app.Sessions != nil {
		sessions, err := s.app.Sessions.List(context.Background())
		if err != nil {
			return nil, rpcError(errorInternal, err.Error())
		}
		ids = make([]string, 0, len(sessions))
		for _, stored := range sessions {
			ids = append(ids, stored.ID)
		}
	} else {
		s.mu.RLock()
		ids = make([]string, 0, len(s.threads))
		for id := range s.threads {
			ids = append(ids, id)
		}
		s.mu.RUnlock()
	}
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
	thread, found, rpcErr := s.readStoredThread(context.Background(), params.ThreadID, params.IncludeTurns)
	if rpcErr != nil {
		return nil, rpcErr
	}
	if !found {
		s.mu.RLock()
		state := s.threads[params.ThreadID]
		s.mu.RUnlock()
		if state == nil {
			return nil, rpcError(errorInvalidParams, "thread not found")
		}
		thread = state.Thread
		if !params.IncludeTurns {
			thread.Turns = []Turn{}
		}
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
	loaded := c.server.threadExists(context.Background(), params.ThreadID)
	if !loaded {
		return map[string]any{"status": "notLoaded"}, nil
	}
	if !c.unsubscribe(params.ThreadID) {
		return map[string]any{"status": "notSubscribed"}, nil
	}
	return map[string]any{"status": "unsubscribed"}, nil
}

func (s *Server) storedThreads(ctx context.Context, includeTurns bool) ([]Thread, *RPCError) {
	if s.app == nil || s.app.Sessions == nil {
		return nil, nil
	}
	sessions, err := s.app.Sessions.List(ctx)
	if err != nil {
		return nil, rpcError(errorInternal, err.Error())
	}
	threads := make([]Thread, 0, len(sessions))
	for _, stored := range sessions {
		thread := s.threadFromSession(ctx, stored, includeTurns)
		threads = append(threads, thread)
	}
	return threads, nil
}

func (s *Server) readStoredThread(ctx context.Context, threadID string, includeTurns bool) (Thread, bool, *RPCError) {
	if s.app == nil || s.app.Sessions == nil {
		return Thread{}, false, nil
	}
	stored, ok, err := s.app.Sessions.Get(ctx, threadID)
	if err != nil {
		return Thread{}, false, rpcError(errorInternal, err.Error())
	}
	if !ok {
		return Thread{}, false, nil
	}
	return s.threadFromSession(ctx, stored, includeTurns), true, nil
}

func (s *Server) threadFromSession(ctx context.Context, stored session.Session, includeTurns bool) Thread {
	created := stored.CreatedAt.Unix()
	updated := stored.UpdatedAt.Unix()
	name := stored.Title
	provider := stored.Provider
	if provider == "" {
		provider = s.app.Config.Provider
	}
	cwd := stored.Workspace
	if cwd == "" {
		cwd = s.app.Config.Workspace
	}
	thread := Thread{
		ID:            stored.ID,
		SessionID:     stored.ID,
		Preview:       stored.Title,
		ModelProvider: provider,
		CreatedAt:     created,
		UpdatedAt:     updated,
		Status:        idleStatus(),
		CWD:           cwd,
		CLIVersion:    s.options.Version,
		Source:        "appServer",
		Turns:         []Turn{},
	}
	if name != "" {
		thread.Name = &name
	}

	s.mu.RLock()
	active := s.threads[stored.ID]
	if active != nil {
		thread.Status = active.Status
		if active.activeCancel != nil {
			thread.Turns = append(thread.Turns, active.Turns...)
		}
	}
	s.mu.RUnlock()

	if includeTurns {
		storedTurns := turnsFromSessionStores(ctx, s.app.Turns, s.app.Items, stored.ID)
		if len(storedTurns) == 0 {
			storedTurns = turnsFromMessages(ctx, s.app.Messages, stored.ID)
		}
		thread.Turns = append(storedTurns, thread.Turns...)
	}
	return thread
}

func turnsFromSessionStores(ctx context.Context, turnStore *session.TurnStore, itemStore *session.ItemStore, sessionID string) []Turn {
	if turnStore == nil || itemStore == nil {
		return nil
	}
	storedTurns, err := turnStore.ListBySession(ctx, sessionID)
	if err != nil {
		return nil
	}
	turns := make([]Turn, 0, len(storedTurns))
	for _, stored := range storedTurns {
		turn := Turn{
			ID:        stored.ID,
			Items:     []any{},
			ItemsView: "full",
			Status:    appServerTurnStatus(stored.Status),
		}
		if !stored.StartedAt.IsZero() {
			started := stored.StartedAt.Unix()
			turn.StartedAt = &started
		}
		if !stored.CompletedAt.IsZero() {
			completed := stored.CompletedAt.Unix()
			turn.CompletedAt = &completed
		}
		if turn.StartedAt != nil && turn.CompletedAt != nil {
			duration := (*turn.CompletedAt - *turn.StartedAt) * 1000
			turn.DurationMs = &duration
		}
		if stored.Error != "" {
			turn.Error = &TurnError{Message: stored.Error}
		}
		items, err := itemStore.ListByTurn(ctx, sessionID, stored.ID)
		if err == nil {
			for _, item := range items {
				turn.Items = append(turn.Items, sessionItem(item))
			}
		}
		turns = append(turns, turn)
	}
	return turns
}

func appServerTurnStatus(status string) string {
	switch status {
	case session.TurnStatusRunning:
		return "inProgress"
	case session.TurnStatusFailed:
		return "failed"
	default:
		return "completed"
	}
}

func sessionItem(item session.Item) map[string]any {
	itemType := "raw"
	switch item.Kind {
	case session.ItemMessage:
		if item.Role == "user" {
			itemType = "userMessage"
		} else {
			itemType = "agentMessage"
		}
	case session.ItemFunctionCall:
		itemType = "toolCall"
	case session.ItemFunctionOut:
		itemType = "toolMessage"
	case session.ItemReasoning:
		itemType = "reasoning"
	case session.ItemCompaction:
		itemType = "compaction"
	}
	out := map[string]any{
		"id":   item.ID,
		"type": itemType,
		"text": item.Text,
	}
	if item.ToolName != "" {
		out["toolName"] = item.ToolName
	}
	if item.ToolCallID != "" {
		out["toolCallId"] = item.ToolCallID
	}
	if len(item.RawJSON) > 0 {
		var raw any
		if err := json.Unmarshal(item.RawJSON, &raw); err == nil {
			out["raw"] = raw
		}
	}
	return out
}

func turnsFromMessages(ctx context.Context, store *messagestore.Store, sessionID string) []Turn {
	if store == nil {
		return nil
	}
	messages, err := store.ListBySession(ctx, sessionID)
	if err != nil {
		return nil
	}
	turns := make([]Turn, 0)
	turnIndex := map[string]int{}
	for _, msg := range messages {
		if msg.RunID == "" {
			continue
		}
		index, ok := turnIndex[msg.RunID]
		if !ok {
			started := msg.CreatedAt.Unix()
			turnIndex[msg.RunID] = len(turns)
			turns = append(turns, Turn{
				ID:        msg.RunID,
				Items:     []any{},
				ItemsView: "full",
				Status:    "completed",
				StartedAt: &started,
			})
			index = len(turns) - 1
		}
		completed := msg.CreatedAt.Unix()
		turns[index].CompletedAt = &completed
		turns[index].Items = append(turns[index].Items, messageItem(msg))
	}
	for i := range turns {
		if turns[i].StartedAt != nil && turns[i].CompletedAt != nil {
			duration := (*turns[i].CompletedAt - *turns[i].StartedAt) * 1000
			turns[i].DurationMs = &duration
		}
	}
	return turns
}

func messageItem(msg messagestore.Message) map[string]any {
	itemType := "userMessage"
	switch msg.Role {
	case messagestore.RoleAssistant:
		itemType = "agentMessage"
	case messagestore.RoleTool:
		itemType = "toolMessage"
	case messagestore.RoleError:
		itemType = "error"
	}
	item := map[string]any{
		"id":   msg.ID,
		"type": itemType,
		"text": msg.Text,
	}
	if msg.ToolName != "" {
		item["toolName"] = msg.ToolName
	}
	if msg.ToolCallID != "" {
		item["toolCallId"] = msg.ToolCallID
	}
	if msg.SourceKind != "" {
		item["sourceKind"] = msg.SourceKind
	}
	if len(msg.RawJSON) > 0 {
		var raw any
		if err := json.Unmarshal(msg.RawJSON, &raw); err == nil {
			item["raw"] = raw
		}
	}
	return item
}

func (s *Server) threadExists(ctx context.Context, threadID string) bool {
	if s.app != nil && s.app.Sessions != nil {
		_, ok, err := s.app.Sessions.Get(ctx, threadID)
		if err == nil && ok {
			return true
		}
	}
	s.mu.RLock()
	defer s.mu.RUnlock()
	return s.threads[threadID] != nil
}
