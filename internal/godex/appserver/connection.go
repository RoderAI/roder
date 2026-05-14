package appserver

import (
	"context"
	"fmt"
	"runtime"
	"strings"
)

func (s *Server) NewConnection(send SendFunc) *Connection {
	conn := &Connection{
		server:     s,
		sendFunc:   send,
		optOut:     make(map[string]struct{}),
		subscribed: make(map[string]struct{}),
	}
	s.mu.Lock()
	s.conns[conn] = struct{}{}
	s.mu.Unlock()
	return conn
}

func (c *Connection) Close() {
	c.server.mu.Lock()
	delete(c.server.conns, c)
	c.server.mu.Unlock()
}

func (c *Connection) HandleJSON(ctx context.Context, data []byte) error {
	msg, err := decodeMessage(data)
	if err != nil {
		return err
	}
	if msg.isEmpty {
		return nil
	}
	if msg.Method == "" {
		return nil
	}
	if !msg.HasID {
		return c.handleNotification(ctx, msg)
	}
	return c.handleRequest(ctx, msg)
}

func (c *Connection) handleNotification(_ context.Context, msg inboundMessage) error {
	if msg.Method == "initialized" {
		return nil
	}
	return nil
}

func (c *Connection) handleRequest(ctx context.Context, msg inboundMessage) error {
	if msg.Method == "initialize" {
		return c.handleInitialize(ctx, msg)
	}
	if !c.isInitialized() {
		return c.sendError(ctx, msg.ID, errorInvalidRequest, "Not initialized")
	}

	result, rpcErr := c.dispatch(ctx, msg)
	if rpcErr != nil {
		return c.send(ctx, Message{ID: msg.ID, Error: rpcErr})
	}
	return c.send(ctx, Message{ID: msg.ID, Result: jsonValue(result)})
}

func (c *Connection) dispatch(ctx context.Context, msg inboundMessage) (any, *RPCError) {
	switch msg.Method {
	case "thread/start":
		return c.handleThreadStart(ctx, msg.Params)
	case "thread/list":
		return c.server.handleThreadList(msg.Params)
	case "thread/loaded/list":
		return c.server.handleThreadLoadedList(msg.Params)
	case "thread/read":
		return c.server.handleThreadRead(msg.Params)
	case "thread/unsubscribe":
		return c.handleThreadUnsubscribe(msg.Params)
	case "thread/goal/get":
		return c.server.handleThreadGoalGet(ctx, msg.Params)
	case "thread/goal/set":
		return c.server.handleThreadGoalSet(ctx, msg.Params)
	case "thread/goal/clear":
		return c.server.handleThreadGoalClear(ctx, msg.Params)
	case "turn/start":
		return c.handleTurnStart(ctx, msg.Params)
	case "turn/steer":
		return c.server.handleTurnSteer(ctx, msg.Params)
	case "turn/interrupt":
		return c.server.handleTurnInterrupt(ctx, msg.Params)
	case "fs/readFile":
		return handleFSReadFile(msg.Params)
	case "fs/writeFile":
		return handleFSWriteFile(msg.Params)
	case "fs/createDirectory":
		return handleFSCreateDirectory(msg.Params)
	case "fs/getMetadata":
		return handleFSGetMetadata(msg.Params)
	case "fs/readDirectory":
		return handleFSReadDirectory(msg.Params)
	case "fs/remove":
		return handleFSRemove(msg.Params)
	case "fs/copy":
		return handleFSCopy(msg.Params)
	case "command/exec":
		return c.server.handleCommandExec(ctx, c, msg.Params)
	case "command/exec/write":
		return c.server.handleCommandWrite(msg.Params)
	case "command/exec/terminate":
		return c.server.handleCommandTerminate(msg.Params)
	case "command/exec/resize":
		return map[string]any{}, nil
	case "model/list":
		return c.server.handleModelList(), nil
	case "skills/list":
		return c.server.handleSkillsList(msg.Params)
	case "skill/read":
		return c.server.handleSkillRead(msg.Params)
	case "skill/setEnabled":
		return c.server.handleSkillSetEnabled(ctx, msg.Params)
	case "mcp/state":
		return c.server.handleMCPState(), nil
	case "mcp/resources/list":
		return c.server.handleMCPResourcesList(), nil
	case "mcp/resource/read":
		return c.server.handleMCPResourceRead(ctx, msg.Params)
	case "lsp/state":
		return c.server.handleLSPState(), nil
	case "lsp/diagnostics":
		return c.server.handleLSPDiagnostics(ctx, msg.Params)
	case "permission/respond":
		return c.server.handlePermissionRespond(ctx, msg.Params)
	default:
		return nil, rpcError(errorMethodNotFound, fmt.Sprintf("Method not found: %s", msg.Method))
	}
}

func (c *Connection) handleInitialize(ctx context.Context, msg inboundMessage) error {
	params, err := decodeParams[InitializeParams](msg.Params)
	if err != nil {
		return c.sendError(ctx, msg.ID, errorInvalidParams, err.Error())
	}
	if strings.TrimSpace(params.ClientInfo.Name) == "" {
		return c.sendError(ctx, msg.ID, errorInvalidParams, "clientInfo.name is required")
	}

	c.mu.Lock()
	if c.initialized {
		c.mu.Unlock()
		return c.sendError(ctx, msg.ID, errorInvalidRequest, "Already initialized")
	}
	c.initialized = true
	c.clientInfo = params.ClientInfo
	for _, method := range params.Capabilities.OptOutNotificationMethods {
		c.optOut[method] = struct{}{}
	}
	c.mu.Unlock()

	result := map[string]any{
		"userAgent":      "gode/" + c.server.options.Version,
		"godeHome":       c.server.app.Config.DataDir,
		"codexHome":      c.server.app.Config.DataDir,
		"workspace":      c.server.app.Config.Workspace,
		"platformFamily": runtime.GOOS,
		"platformOs":     runtime.GOOS,
		"capabilities":   protocolCapabilities(),
	}
	return c.send(ctx, Message{ID: msg.ID, Result: jsonValue(result)})
}

func (c *Connection) isInitialized() bool {
	c.mu.RLock()
	defer c.mu.RUnlock()
	return c.initialized
}

func (c *Connection) subscribe(threadID string) {
	c.mu.Lock()
	c.subscribed[threadID] = struct{}{}
	c.mu.Unlock()
}

func (c *Connection) unsubscribe(threadID string) bool {
	c.mu.Lock()
	defer c.mu.Unlock()
	if _, ok := c.subscribed[threadID]; !ok {
		return false
	}
	delete(c.subscribed, threadID)
	return true
}

func (c *Connection) isSubscribed(threadID string) bool {
	c.mu.RLock()
	defer c.mu.RUnlock()
	_, ok := c.subscribed[threadID]
	return ok
}

func (c *Connection) sendError(ctx context.Context, id any, code int, message string) error {
	return c.send(ctx, Message{ID: id, Error: rpcError(code, message)})
}

func (c *Connection) sendNotification(ctx context.Context, method string, params any) error {
	c.mu.RLock()
	_, optedOut := c.optOut[method]
	c.mu.RUnlock()
	if optedOut {
		return nil
	}
	return c.send(ctx, Message{Method: method, Params: jsonValue(params)})
}

func (c *Connection) send(ctx context.Context, msg Message) error {
	if c.sendFunc == nil {
		return nil
	}
	c.sendMu.Lock()
	defer c.sendMu.Unlock()
	return c.sendFunc(ctx, msg)
}
