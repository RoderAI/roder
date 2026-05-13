package acp

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/google/uuid"
	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/agent"
	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/tools/builtin"
)

type SendFunc func(context.Context, Message) error

type Options struct {
	Version string
	Run     func(context.Context, agent.RunRequest) (agent.RunResult, error)
}

type Server struct {
	app     *godex.App
	options Options
	run     func(context.Context, agent.RunRequest) (agent.RunResult, error)

	mu       sync.RWMutex
	sessions map[string]*sessionState
}

type Connection struct {
	server   *Server
	sendFunc SendFunc

	mu          sync.RWMutex
	sendMu      sync.Mutex
	initialized bool
	clientInfo  implementation
	pending     map[string]chan inboundMessage
}

type sessionState struct {
	id           string
	cwd          string
	title        string
	createdAt    time.Time
	updatedAt    time.Time
	activeCancel context.CancelFunc
	activeRunID  string
}

func New(app *godex.App, options Options) *Server {
	if options.Version == "" {
		options.Version = "dev"
	}
	run := options.Run
	if run == nil && app != nil {
		run = app.Run
	}
	return &Server{
		app:      app,
		options:  options,
		run:      run,
		sessions: make(map[string]*sessionState),
	}
}

func (s *Server) NewConnection(send SendFunc) *Connection {
	return &Connection{
		server:   s,
		sendFunc: send,
		pending:  make(map[string]chan inboundMessage),
	}
}

func (c *Connection) HandleJSON(ctx context.Context, data []byte) error {
	msg, decodeErr := decodeMessage(data)
	if decodeErr != nil {
		return c.send(ctx, errorMessage(nil, decodeErr))
	}
	if msg.IsResponse {
		c.handleResponse(msg)
		return nil
	}
	if !msg.HasID {
		return c.handleNotification(ctx, msg)
	}
	if msg.Method == "session/prompt" {
		go c.handlePromptRequest(ctx, msg)
		return nil
	}
	return c.handleRequest(ctx, msg)
}

func (c *Connection) handleNotification(ctx context.Context, msg inboundMessage) error {
	switch msg.Method {
	case "session/cancel":
		params, err := decodeParams[cancelParams](msg.Params)
		if err != nil {
			return nil
		}
		c.server.cancelSession(params.SessionID)
	case "_ping":
		return nil
	default:
		if strings.HasPrefix(msg.Method, "_") {
			return nil
		}
	}
	_ = ctx
	return nil
}

func (c *Connection) handleRequest(ctx context.Context, msg inboundMessage) error {
	if msg.Method == "initialize" {
		return c.handleInitialize(ctx, msg)
	}
	if !c.isInitialized() {
		return c.send(ctx, errorMessage(msg.ID, rpcError(errorInvalidRequest, "Not initialized")))
	}

	result, rpcErr := c.dispatch(ctx, msg)
	if rpcErr != nil {
		return c.send(ctx, errorMessage(msg.ID, rpcErr))
	}
	return c.send(ctx, responseMessage(msg.ID, result))
}

func (c *Connection) dispatch(ctx context.Context, msg inboundMessage) (any, *RPCError) {
	switch msg.Method {
	case "authenticate":
		return map[string]any{}, nil
	case "session/new":
		return c.server.handleSessionNew(ctx, msg.Params)
	case "session/list":
		return c.server.handleSessionList(msg.Params)
	case "session/close":
		return c.server.handleSessionClose(msg.Params)
	default:
		if strings.HasPrefix(msg.Method, "_") {
			return map[string]any{}, nil
		}
		return nil, rpcError(errorMethodNotFound, fmt.Sprintf("Method not found: %s", msg.Method))
	}
}

func (c *Connection) handleInitialize(ctx context.Context, msg inboundMessage) error {
	if err := requiredParamFields(msg.Params, "protocolVersion"); err != nil {
		return c.send(ctx, errorMessage(msg.ID, rpcError(errorInvalidParams, err.Error())))
	}
	params, err := decodeParams[initializeParams](msg.Params)
	if err != nil {
		return c.send(ctx, errorMessage(msg.ID, rpcError(errorInvalidParams, err.Error())))
	}

	c.mu.Lock()
	if c.initialized {
		c.mu.Unlock()
		return c.send(ctx, errorMessage(msg.ID, rpcError(errorInvalidRequest, "Already initialized")))
	}
	c.initialized = true
	c.clientInfo = params.ClientInfo
	c.mu.Unlock()

	return c.send(ctx, responseMessage(msg.ID, initializeResult(c.server.options.Version)))
}

func (c *Connection) handlePromptRequest(ctx context.Context, msg inboundMessage) {
	if !c.isInitialized() {
		_ = c.send(ctx, errorMessage(msg.ID, rpcError(errorInvalidRequest, "Not initialized")))
		return
	}
	result, rpcErr := c.server.runPrompt(ctx, c, msg.Params)
	if rpcErr != nil {
		_ = c.send(ctx, errorMessage(msg.ID, rpcErr))
		return
	}
	_ = c.send(ctx, responseMessage(msg.ID, result))
}

func (c *Connection) isInitialized() bool {
	c.mu.RLock()
	defer c.mu.RUnlock()
	return c.initialized
}

func (s *Server) handleSessionNew(ctx context.Context, raw json.RawMessage) (any, *RPCError) {
	if err := requiredParamFields(raw, "cwd", "mcpServers"); err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	params, err := decodeParams[newSessionParams](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	if err := requireAbsolutePath("cwd", params.CWD); err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	if rpcErr := s.connectMCPServers(ctx, params.MCPServers); rpcErr != nil {
		return nil, rpcErr
	}

	now := time.Now().UTC()
	session := &sessionState{
		id:        uuid.NewString(),
		cwd:       params.CWD,
		title:     filepath.Base(params.CWD),
		createdAt: now,
		updatedAt: now,
	}
	s.mu.Lock()
	s.sessions[session.id] = session
	s.mu.Unlock()
	return map[string]any{"sessionId": session.id}, nil
}

func (s *Server) connectMCPServers(ctx context.Context, servers []mcpServer) *RPCError {
	configs, names, err := validateMCPServers(servers)
	if err != nil {
		return rpcError(errorInvalidParams, err.Error())
	}
	if len(configs) == 0 {
		return nil
	}
	if s.app == nil || s.app.MCP == nil || s.app.Tools == nil {
		return rpcError(errorInternal, "mcp is not available")
	}
	for i, cfg := range configs {
		if err := s.app.MCP.AddStdioServer(ctx, names[i], cfg); err != nil {
			return rpcError(errorInternal, err.Error())
		}
	}
	builtin.RegisterMCP(s.app.Tools, s.app.MCP)
	return nil
}

func (s *Server) handleSessionList(raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[listSessionsParams](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	if params.Cursor != "" {
		return nil, rpcError(errorInvalidParams, "cursor is invalid")
	}
	if params.CWD != "" {
		if err := requireAbsolutePath("cwd", params.CWD); err != nil {
			return nil, rpcError(errorInvalidParams, err.Error())
		}
	}

	s.mu.RLock()
	sessions := make([]sessionInfo, 0, len(s.sessions))
	for _, session := range s.sessions {
		if params.CWD != "" && session.cwd != params.CWD {
			continue
		}
		sessions = append(sessions, sessionInfo{
			SessionID: session.id,
			CWD:       session.cwd,
			Title:     session.title,
			UpdatedAt: formatACPTime(session.updatedAt),
		})
	}
	s.mu.RUnlock()
	sort.Slice(sessions, func(i, j int) bool {
		return sessions[i].UpdatedAt > sessions[j].UpdatedAt
	})
	return map[string]any{"sessions": sessions}, nil
}

func (s *Server) handleSessionClose(raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[closeSessionParams](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	if strings.TrimSpace(params.SessionID) == "" {
		return nil, rpcError(errorInvalidParams, "sessionId is required")
	}
	s.mu.Lock()
	session := s.sessions[params.SessionID]
	if session == nil {
		s.mu.Unlock()
		return nil, rpcError(errorNotFound, "session not found")
	}
	cancel := session.activeCancel
	delete(s.sessions, params.SessionID)
	s.mu.Unlock()
	if cancel != nil {
		cancel()
	}
	return map[string]any{}, nil
}

func (s *Server) cancelSession(sessionID string) {
	s.mu.RLock()
	session := s.sessions[sessionID]
	var cancel context.CancelFunc
	if session != nil {
		cancel = session.activeCancel
	}
	s.mu.RUnlock()
	if cancel != nil {
		cancel()
	}
}

func (s *Server) runPrompt(ctx context.Context, conn *Connection, raw json.RawMessage) (any, *RPCError) {
	if s.app == nil || s.run == nil {
		return nil, rpcError(errorInternal, "agent runner is not available")
	}
	if err := requiredParamFields(raw, "sessionId", "prompt"); err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	params, err := decodeParams[promptParams](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	prompt, err := promptToText(params.Prompt)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	runID := uuid.NewString()
	runCtx, cancel := context.WithCancel(ctx)
	defer cancel()

	s.mu.Lock()
	session := s.sessions[params.SessionID]
	if session == nil {
		s.mu.Unlock()
		return nil, rpcError(errorNotFound, "session not found")
	}
	if session.activeCancel != nil {
		s.mu.Unlock()
		return nil, rpcError(errorInvalidRequest, "session already has an active prompt")
	}
	session.activeCancel = cancel
	session.activeRunID = runID
	session.updatedAt = time.Now().UTC()
	s.mu.Unlock()

	subCtx, cancelSub := context.WithCancel(context.Background())
	events := s.app.Bus.Subscribe(subCtx, eventbus.Filter{SessionID: params.SessionID, RunID: runID})
	defer cancelSub()

	resultCh := make(chan agent.RunResult, 1)
	errCh := make(chan error, 1)
	go func() {
		result, err := s.run(runCtx, agent.RunRequest{SessionID: params.SessionID, RunID: runID, Prompt: prompt})
		if err != nil {
			errCh <- err
			return
		}
		resultCh <- result
	}()

	sentText := false
	var runResult agent.RunResult
	var runErr error
	done := false
	for !done {
		select {
		case ev := <-events:
			if s.handleRunEvent(runCtx, conn, params.SessionID, ev) {
				sentText = true
			}
		case runResult = <-resultCh:
			done = true
		case runErr = <-errCh:
			done = true
		case <-runCtx.Done():
			runErr = runCtx.Err()
			done = true
		}
	}
drain:
	for {
		select {
		case ev := <-events:
			if s.handleRunEvent(runCtx, conn, params.SessionID, ev) {
				sentText = true
			}
		default:
			break drain
		}
	}

	s.mu.Lock()
	if session := s.sessions[params.SessionID]; session != nil && session.activeRunID == runID {
		session.activeCancel = nil
		session.activeRunID = ""
		session.updatedAt = time.Now().UTC()
		if session.title == "" && strings.TrimSpace(prompt) != "" {
			session.title = firstLine(prompt)
		}
	}
	s.mu.Unlock()

	if errors.Is(runErr, context.Canceled) || errors.Is(runErr, context.DeadlineExceeded) || runCtx.Err() != nil {
		return map[string]any{"stopReason": "cancelled"}, nil
	}
	if runErr != nil {
		return nil, rpcError(errorInternal, runErr.Error())
	}
	if !sentText && strings.TrimSpace(runResult.FinalText) != "" {
		_ = conn.sendNotification(context.Background(), "session/update", map[string]any{
			"sessionId": params.SessionID,
			"update": map[string]any{
				"sessionUpdate": "agent_message_chunk",
				"content":       map[string]any{"type": "text", "text": runResult.FinalText},
			},
		})
	}
	return map[string]any{"stopReason": "end_turn"}, nil
}

func (s *Server) handleRunEvent(ctx context.Context, conn *Connection, sessionID string, ev eventbus.Event) bool {
	switch ev.Kind {
	case eventbus.KindAssistantDelta:
		var payload struct {
			Text string `json:"text"`
		}
		_ = ev.DecodePayload(&payload)
		if payload.Text == "" {
			return false
		}
		_ = conn.sendNotification(ctx, "session/update", map[string]any{
			"sessionId": sessionID,
			"update": map[string]any{
				"sessionUpdate": "agent_message_chunk",
				"content":       map[string]any{"type": "text", "text": payload.Text},
			},
		})
		return true
	case eventbus.KindToolRequested:
		payload := decodeToolPayload(ev)
		_ = conn.sendNotification(ctx, "session/update", map[string]any{
			"sessionId": sessionID,
			"update": map[string]any{
				"sessionUpdate": "tool_call",
				"toolCallId":    payload.ToolCallID,
				"title":         payload.Title(),
				"kind":          toolKind(payload.Tool),
				"status":        "pending",
				"rawInput":      payload.Input,
			},
		})
	case eventbus.KindToolStarted:
		payload := decodeToolPayload(ev)
		_ = conn.sendNotification(ctx, "session/update", map[string]any{
			"sessionId": sessionID,
			"update": map[string]any{
				"sessionUpdate": "tool_call_update",
				"toolCallId":    payload.ToolCallID,
				"status":        "in_progress",
			},
		})
	case eventbus.KindToolCompleted:
		payload := decodeToolPayload(ev)
		update := map[string]any{
			"sessionUpdate": "tool_call_update",
			"toolCallId":    payload.ToolCallID,
			"status":        "completed",
			"rawOutput":     payload.Raw,
		}
		if payload.Text != "" {
			update["content"] = []map[string]any{{
				"type":    "content",
				"content": map[string]any{"type": "text", "text": payload.Text},
			}}
		}
		_ = conn.sendNotification(ctx, "session/update", map[string]any{"sessionId": sessionID, "update": update})
	case eventbus.KindToolFailed:
		payload := decodeToolPayload(ev)
		text := payload.Error
		if text == "" {
			text = "Tool failed"
		}
		_ = conn.sendNotification(ctx, "session/update", map[string]any{
			"sessionId": sessionID,
			"update": map[string]any{
				"sessionUpdate": "tool_call_update",
				"toolCallId":    payload.ToolCallID,
				"status":        "failed",
				"content": []map[string]any{{
					"type":    "content",
					"content": map[string]any{"type": "text", "text": text},
				}},
			},
		})
	case eventbus.KindPermissionRequested:
		s.handlePermissionRequested(ctx, conn, sessionID, ev)
	}
	return false
}

func (s *Server) handlePermissionRequested(ctx context.Context, conn *Connection, sessionID string, ev eventbus.Event) {
	payload := decodeToolPayload(ev)
	requestID := "permission-" + uuid.NewString()
	ch := conn.registerPending(requestID)
	if ch == nil {
		return
	}
	params := map[string]any{
		"sessionId": sessionID,
		"toolCall": map[string]any{
			"toolCallId": payload.ToolCallID,
			"title":      payload.Title(),
			"kind":       toolKind(payload.Tool),
			"status":     "pending",
			"rawInput":   payload.Input,
		},
		"options": []map[string]any{
			{"optionId": "allow_once", "name": "Allow once", "kind": "allow_once"},
			{"optionId": "reject_once", "name": "Reject", "kind": "reject_once"},
		},
	}
	if err := conn.send(ctx, requestMessage(requestID, "session/request_permission", params)); err != nil {
		conn.removePending(requestID)
		return
	}
	go func() {
		approved := false
		select {
		case response := <-ch:
			approved = permissionApproved(response.Result)
		case <-ctx.Done():
		}
		s.app.Bus.Publish(context.Background(), eventbus.Event{
			Kind:          eventbus.KindPermissionResponded,
			Source:        eventbus.SourceSystem,
			SessionID:     ev.SessionID,
			RunID:         ev.RunID,
			CorrelationID: ev.CorrelationID,
			Payload:       map[string]any{"approved": approved},
		})
	}()
}

func permissionApproved(raw json.RawMessage) bool {
	var result struct {
		Outcome struct {
			Outcome  string `json:"outcome"`
			OptionID string `json:"optionId"`
		} `json:"outcome"`
	}
	if err := json.Unmarshal(raw, &result); err != nil {
		return false
	}
	return result.Outcome.Outcome == "selected" && strings.HasPrefix(result.Outcome.OptionID, "allow")
}

type toolPayload struct {
	ToolCallID  string         `json:"tool_call_id"`
	ToolCallID2 string         `json:"toolCallId"`
	Tool        string         `json:"tool"`
	Description string         `json:"description"`
	Input       map[string]any `json:"input"`
	Text        string         `json:"text"`
	Error       string         `json:"error"`
	Raw         map[string]any
}

func (p toolPayload) Title() string {
	if p.Description != "" {
		return p.Description
	}
	if p.Tool != "" {
		return p.Tool
	}
	return "Tool call"
}

func decodeToolPayload(ev eventbus.Event) toolPayload {
	var payload toolPayload
	data, _ := json.Marshal(ev.Payload)
	_ = json.Unmarshal(data, &payload)
	if payload.ToolCallID == "" {
		payload.ToolCallID = payload.ToolCallID2
	}
	if payload.ToolCallID == "" {
		payload.ToolCallID = ev.ID
	}
	_ = json.Unmarshal(data, &payload.Raw)
	return payload
}

func (c *Connection) sendNotification(ctx context.Context, method string, params any) error {
	return c.send(ctx, notificationMessage(method, params))
}

func (c *Connection) send(ctx context.Context, msg Message) error {
	if msg.JSONRPC == "" {
		msg.JSONRPC = jsonrpcVersion
	}
	if c.sendFunc == nil {
		return nil
	}
	c.sendMu.Lock()
	defer c.sendMu.Unlock()
	return c.sendFunc(ctx, msg)
}

func (c *Connection) registerPending(id any) <-chan inboundMessage {
	key := idKey(id)
	ch := make(chan inboundMessage, 1)
	c.mu.Lock()
	if _, exists := c.pending[key]; exists {
		c.mu.Unlock()
		return nil
	}
	c.pending[key] = ch
	c.mu.Unlock()
	return ch
}

func (c *Connection) removePending(id any) {
	key := idKey(id)
	c.mu.Lock()
	delete(c.pending, key)
	c.mu.Unlock()
}

func (c *Connection) handleResponse(msg inboundMessage) {
	key := idKey(msg.ID)
	c.mu.Lock()
	ch := c.pending[key]
	delete(c.pending, key)
	c.mu.Unlock()
	if ch != nil {
		ch <- msg
	}
}

func requireAbsolutePath(name string, value string) error {
	if strings.TrimSpace(value) == "" {
		return fmt.Errorf("%s is required", name)
	}
	if !filepath.IsAbs(value) {
		return fmt.Errorf("%s must be absolute", name)
	}
	return nil
}

func firstLine(text string) string {
	text = strings.TrimSpace(text)
	if text == "" {
		return ""
	}
	if idx := strings.IndexByte(text, '\n'); idx >= 0 {
		text = text[:idx]
	}
	if len(text) > 80 {
		text = text[:80]
	}
	return text
}
