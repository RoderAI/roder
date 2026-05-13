package acp

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sync"
	"testing"
	"time"

	mcpsdk "github.com/modelcontextprotocol/go-sdk/mcp"
	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/agent"
	"github.com/pandelisz/gode/internal/godex/eventbus"
)

func TestStdioEndToEndInitializeSessionListAndPrompt(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	app := newACPTestApp(t)
	server := New(app, Options{Version: "test"})

	inputReader, inputWriter := io.Pipe()
	outputReader, outputWriter := io.Pipe()
	errCh := make(chan error, 1)
	go func() {
		errCh <- server.ServeStdio(ctx, inputReader, outputWriter)
	}()
	t.Cleanup(func() {
		_ = inputWriter.Close()
		_ = outputReader.Close()
		_ = outputWriter.Close()
		if err := <-errCh; err != nil && !errors.Is(err, io.ErrClosedPipe) {
			t.Fatalf("serve stdio: %v", err)
		}
	})

	scanner := bufio.NewScanner(outputReader)
	writeRPCLine(t, inputWriter, map[string]any{
		"jsonrpc": "2.0",
		"id":      1,
		"method":  "initialize",
		"params": map[string]any{
			"protocolVersion": 1,
			"clientCapabilities": map[string]any{
				"fs":       map[string]any{"readTextFile": false, "writeTextFile": false},
				"terminal": false,
			},
			"clientInfo": map[string]any{"name": "gode-acp-test", "version": "0.1.0"},
		},
	})
	init := readRPCMessage(t, scanner)
	assertResponseID(t, init, float64(1))
	initResult := objectField(t, init, "result")
	if initResult["protocolVersion"] != float64(1) {
		t.Fatalf("protocolVersion = %#v", initResult["protocolVersion"])
	}
	agentInfo := objectField(t, initResult, "agentInfo")
	if agentInfo["name"] != "gode" {
		t.Fatalf("agentInfo = %#v", agentInfo)
	}
	caps := objectField(t, initResult, "agentCapabilities")
	if caps["loadSession"] != false {
		t.Fatalf("loadSession capability = %#v", caps["loadSession"])
	}
	promptCaps := objectField(t, caps, "promptCapabilities")
	if promptCaps["image"] != false || promptCaps["audio"] != false || promptCaps["embeddedContext"] != false {
		t.Fatalf("prompt capabilities should only advertise baseline content support: %#v", promptCaps)
	}
	sessionCaps := objectField(t, caps, "sessionCapabilities")
	if _, ok := sessionCaps["list"]; !ok {
		t.Fatalf("sessionCapabilities.list missing: %#v", sessionCaps)
	}
	if _, ok := sessionCaps["close"]; !ok {
		t.Fatalf("sessionCapabilities.close missing: %#v", sessionCaps)
	}

	writeRPCLine(t, inputWriter, map[string]any{
		"jsonrpc": "2.0",
		"id":      2,
		"method":  "session/new",
		"params": map[string]any{
			"cwd":        app.Config.Workspace,
			"mcpServers": []any{},
		},
	})
	newSession := readRPCMessage(t, scanner)
	assertResponseID(t, newSession, float64(2))
	sessionID := stringField(t, objectField(t, newSession, "result"), "sessionId")
	if sessionID == "" {
		t.Fatal("sessionId is empty")
	}

	writeRPCLine(t, inputWriter, map[string]any{
		"jsonrpc": "2.0",
		"id":      3,
		"method":  "session/list",
		"params":  map[string]any{"cwd": app.Config.Workspace},
	})
	list := readRPCMessage(t, scanner)
	assertResponseID(t, list, float64(3))
	sessions := sliceField(t, objectField(t, list, "result"), "sessions")
	if len(sessions) != 1 {
		t.Fatalf("session/list returned %d sessions: %#v", len(sessions), sessions)
	}
	if got := stringField(t, sessions[0].(map[string]any), "sessionId"); got != sessionID {
		t.Fatalf("listed sessionId = %q, want %q", got, sessionID)
	}

	writeRPCLine(t, inputWriter, map[string]any{
		"jsonrpc": "2.0",
		"id":      4,
		"method":  "session/prompt",
		"params": map[string]any{
			"sessionId": sessionID,
			"prompt": []map[string]any{
				{"type": "text", "text": "hello"},
				{"type": "resource_link", "uri": "file://" + filepath.Join(app.Config.Workspace, "README.md"), "name": "README.md"},
			},
		},
	})

	var sawAgentChunk bool
	var promptResponse map[string]any
	for promptResponse == nil {
		msg := readRPCMessage(t, scanner)
		if msg["method"] == "session/update" {
			params := objectField(t, msg, "params")
			if params["sessionId"] != sessionID {
				t.Fatalf("session/update sessionId = %#v, want %q", params["sessionId"], sessionID)
			}
			update := objectField(t, params, "update")
			if update["sessionUpdate"] == "agent_message_chunk" {
				content := objectField(t, update, "content")
				if content["type"] != "text" || content["text"] != "mock response" {
					t.Fatalf("agent content = %#v", content)
				}
				sawAgentChunk = true
			}
			continue
		}
		if idMatches(msg["id"], float64(4)) {
			promptResponse = msg
		}
	}
	if !sawAgentChunk {
		t.Fatal("missing agent_message_chunk session/update")
	}
	if stop := objectField(t, promptResponse, "result")["stopReason"]; stop != "end_turn" {
		t.Fatalf("stopReason = %#v", stop)
	}
}

func TestPromptRejectsUnsupportedContentTypes(t *testing.T) {
	app := newACPTestApp(t)
	conn, messages := newInitializedConnection(t, app, nil)
	sessionID := createACPSession(t, conn, messages, app.Config.Workspace)

	sendRPC(t, conn, map[string]any{
		"jsonrpc": "2.0",
		"id":      10,
		"method":  "session/prompt",
		"params": map[string]any{
			"sessionId": sessionID,
			"prompt": []map[string]any{
				{"type": "image", "mimeType": "image/png", "data": "AAAA"},
			},
		},
	})

	msg := waitForResponse(t, messages, float64(10))
	errObj := objectField(t, msg, "error")
	if errObj["code"] != float64(-32602) {
		t.Fatalf("error code = %#v", errObj["code"])
	}
	if errObj["message"] == "" {
		t.Fatalf("error message missing: %#v", errObj)
	}
}

func TestSessionCancelReturnsCancelledStopReason(t *testing.T) {
	app := newACPTestApp(t)
	runStarted := make(chan struct{})
	serverRun := func(ctx context.Context, req agent.RunRequest) (agent.RunResult, error) {
		close(runStarted)
		<-ctx.Done()
		return agent.RunResult{SessionID: req.SessionID, RunID: req.RunID}, ctx.Err()
	}
	conn, messages := newInitializedConnection(t, app, serverRun)
	sessionID := createACPSession(t, conn, messages, app.Config.Workspace)

	sendRPC(t, conn, map[string]any{
		"jsonrpc": "2.0",
		"id":      20,
		"method":  "session/prompt",
		"params": map[string]any{
			"sessionId": sessionID,
			"prompt":    []map[string]any{{"type": "text", "text": "cancel me"}},
		},
	})
	select {
	case <-runStarted:
	case <-time.After(2 * time.Second):
		t.Fatal("prompt run did not start")
	}
	sendRPC(t, conn, map[string]any{
		"jsonrpc": "2.0",
		"method":  "session/cancel",
		"params":  map[string]any{"sessionId": sessionID},
	})

	msg := waitForResponse(t, messages, float64(20))
	if stop := objectField(t, msg, "result")["stopReason"]; stop != "cancelled" {
		t.Fatalf("stopReason = %#v", stop)
	}
}

func TestToolUpdatesAndPermissionBridgeUseACPShapes(t *testing.T) {
	app := newACPTestApp(t)
	permissionCorrelationID := "permission-1"
	serverRun := func(ctx context.Context, req agent.RunRequest) (agent.RunResult, error) {
		app.Bus.Publish(ctx, eventbus.Event{
			Kind:      eventbus.KindToolRequested,
			Source:    eventbus.SourceProvider,
			SessionID: req.SessionID,
			RunID:     req.RunID,
			Payload: map[string]any{
				"tool_call_id": "call-1",
				"tool":         "shell.exec",
				"input":        map[string]any{"command": "true"},
			},
		})
		app.Bus.Publish(ctx, eventbus.Event{
			Kind:          eventbus.KindPermissionRequested,
			Source:        eventbus.SourceTool,
			SessionID:     req.SessionID,
			RunID:         req.RunID,
			CorrelationID: permissionCorrelationID,
			Payload: map[string]any{
				"tool_call_id": "call-1",
				"tool":         "shell.exec",
				"description":  "Run command",
				"input":        map[string]any{"command": "true"},
			},
		})

		if !waitForPermissionResponse(ctx, app.Bus, permissionCorrelationID) {
			return agent.RunResult{}, ctx.Err()
		}
		app.Bus.Publish(ctx, eventbus.Event{
			Kind:      eventbus.KindToolStarted,
			Source:    eventbus.SourceTool,
			SessionID: req.SessionID,
			RunID:     req.RunID,
			Payload: map[string]any{
				"tool_call_id": "call-1",
				"tool":         "shell.exec",
			},
		})
		app.Bus.Publish(ctx, eventbus.Event{
			Kind:      eventbus.KindToolCompleted,
			Source:    eventbus.SourceTool,
			SessionID: req.SessionID,
			RunID:     req.RunID,
			Payload: map[string]any{
				"tool_call_id": "call-1",
				"tool":         "shell.exec",
				"text":         "ok",
			},
		})
		app.Bus.Publish(ctx, eventbus.Event{
			Kind:      eventbus.KindAssistantDelta,
			Source:    eventbus.SourceProvider,
			SessionID: req.SessionID,
			RunID:     req.RunID,
			Payload:   map[string]any{"text": "done"},
		})
		return agent.RunResult{SessionID: req.SessionID, RunID: req.RunID, FinalText: "done"}, nil
	}

	conn, messages := newInitializedConnection(t, app, serverRun)
	sessionID := createACPSession(t, conn, messages, app.Config.Workspace)
	sendRPC(t, conn, map[string]any{
		"jsonrpc": "2.0",
		"id":      30,
		"method":  "session/prompt",
		"params": map[string]any{
			"sessionId": sessionID,
			"prompt":    []map[string]any{{"type": "text", "text": "use a tool"}},
		},
	})

	permissionRequest := waitForMethod(t, messages, "session/request_permission")
	if permissionRequest["id"] == nil {
		t.Fatalf("permission request id missing: %#v", permissionRequest)
	}
	permissionParams := objectField(t, permissionRequest, "params")
	if permissionParams["sessionId"] != sessionID {
		t.Fatalf("permission sessionId = %#v", permissionParams["sessionId"])
	}
	toolCall := objectField(t, permissionParams, "toolCall")
	if toolCall["toolCallId"] != "call-1" {
		t.Fatalf("permission toolCall = %#v", toolCall)
	}
	sendRPC(t, conn, map[string]any{
		"jsonrpc": "2.0",
		"id":      permissionRequest["id"],
		"result":  map[string]any{"outcome": map[string]any{"outcome": "selected", "optionId": "allow_once"}},
	})

	waitForResponse(t, messages, float64(30))
	for _, expected := range []string{"tool_call", "tool_call_update", "agent_message_chunk"} {
		if !hasSessionUpdate(t, messages, expected) {
			t.Fatalf("missing session update %q in %#v", expected, messages.snapshot())
		}
	}
}

func TestSessionNewConnectsStdioMCPServerAndRegistersTools(t *testing.T) {
	app := newACPTestApp(t)
	conn, messages := newInitializedConnection(t, app, nil)
	helperPath, err := filepath.Abs(os.Args[0])
	if err != nil {
		t.Fatalf("helper path: %v", err)
	}

	sendRPC(t, conn, map[string]any{
		"jsonrpc": "2.0",
		"id":      40,
		"method":  "session/new",
		"params": map[string]any{
			"cwd": app.Config.Workspace,
			"mcpServers": []map[string]any{
				{
					"name":    "helper",
					"command": helperPath,
					"args":    []string{"-test.run=TestMCPHelperProcess", "--"},
					"env":     []map[string]any{{"name": "GODE_ACP_MCP_HELPER", "value": "1"}},
				},
			},
		},
	})
	waitForResponse(t, messages, float64(40))

	for _, spec := range app.Tools.Specs() {
		if spec.Name == "mcp_helper_echo" {
			return
		}
	}
	t.Fatalf("mcp helper tool was not registered: %#v", app.Tools.Specs())
}

func TestMCPHelperProcess(t *testing.T) {
	if os.Getenv("GODE_ACP_MCP_HELPER") != "1" {
		return
	}
	server := mcpsdk.NewServer(&mcpsdk.Implementation{Name: "gode-acp-test-helper", Version: "test"}, nil)
	type echoArgs struct {
		Text string `json:"text" jsonschema:"text to echo"`
	}
	mcpsdk.AddTool(server, &mcpsdk.Tool{Name: "echo", Description: "echo text"}, func(_ context.Context, _ *mcpsdk.CallToolRequest, args echoArgs) (*mcpsdk.CallToolResult, any, error) {
		return &mcpsdk.CallToolResult{Content: []mcpsdk.Content{&mcpsdk.TextContent{Text: args.Text}}}, nil, nil
	})
	if err := server.Run(context.Background(), &mcpsdk.StdioTransport{}); err != nil {
		_, _ = fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	os.Exit(0)
}

func newACPTestApp(t *testing.T) *godex.App {
	t.Helper()
	app, err := godex.New(context.Background(), godex.Config{
		Workspace:   filepath.Join(t.TempDir(), "workspace"),
		DataDir:     t.TempDir(),
		Provider:    "mock",
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("new app: %v", err)
	}
	t.Cleanup(func() {
		_ = app.Close(context.Background())
	})
	return app
}

func newInitializedConnection(t *testing.T, app *godex.App, run func(context.Context, agent.RunRequest) (agent.RunResult, error)) (*Connection, *messageSink) {
	t.Helper()
	sink := &messageSink{}
	options := Options{Version: "test"}
	if run != nil {
		options.Run = run
	}
	conn := New(app, options).NewConnection(func(_ context.Context, msg Message) error {
		sink.append(msg)
		return nil
	})
	sendRPC(t, conn, map[string]any{
		"jsonrpc": "2.0",
		"id":      1,
		"method":  "initialize",
		"params":  map[string]any{"protocolVersion": 1, "clientInfo": map[string]any{"name": "test"}},
	})
	waitForResponse(t, sink, float64(1))
	return conn, sink
}

func createACPSession(t *testing.T, conn *Connection, messages *messageSink, cwd string) string {
	t.Helper()
	sendRPC(t, conn, map[string]any{
		"jsonrpc": "2.0",
		"id":      2,
		"method":  "session/new",
		"params":  map[string]any{"cwd": cwd, "mcpServers": []any{}},
	})
	return stringField(t, objectField(t, waitForResponse(t, messages, float64(2)), "result"), "sessionId")
}

func writeRPCLine(t *testing.T, w io.Writer, msg map[string]any) {
	t.Helper()
	data, err := json.Marshal(msg)
	if err != nil {
		t.Fatalf("marshal rpc: %v", err)
	}
	if _, err := w.Write(append(data, '\n')); err != nil {
		t.Fatalf("write rpc: %v", err)
	}
}

func sendRPC(t *testing.T, conn *Connection, msg map[string]any) {
	t.Helper()
	data, err := json.Marshal(msg)
	if err != nil {
		t.Fatalf("marshal rpc: %v", err)
	}
	if err := conn.HandleJSON(context.Background(), data); err != nil {
		t.Fatalf("handle rpc: %v", err)
	}
}

func readRPCMessage(t *testing.T, scanner *bufio.Scanner) map[string]any {
	t.Helper()
	if !scanner.Scan() {
		t.Fatalf("missing rpc message: %v", scanner.Err())
	}
	var msg map[string]any
	if err := json.Unmarshal(scanner.Bytes(), &msg); err != nil {
		t.Fatalf("unmarshal rpc %q: %v", scanner.Text(), err)
	}
	if msg["jsonrpc"] != "2.0" {
		t.Fatalf("jsonrpc = %#v in %#v", msg["jsonrpc"], msg)
	}
	return msg
}

type messageSink struct {
	mu       sync.Mutex
	messages []map[string]any
}

func (s *messageSink) append(msg Message) {
	data, _ := json.Marshal(msg)
	var out map[string]any
	_ = json.Unmarshal(data, &out)
	s.mu.Lock()
	s.messages = append(s.messages, out)
	s.mu.Unlock()
}

func (s *messageSink) snapshot() []map[string]any {
	s.mu.Lock()
	defer s.mu.Unlock()
	out := make([]map[string]any, len(s.messages))
	copy(out, s.messages)
	return out
}

func waitForResponse(t *testing.T, sink *messageSink, id any) map[string]any {
	t.Helper()
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		for _, msg := range sink.snapshot() {
			if idMatches(msg["id"], id) && (msg["result"] != nil || msg["error"] != nil) {
				return msg
			}
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("missing response id %v in %#v", id, sink.snapshot())
	return nil
}

func waitForMethod(t *testing.T, sink *messageSink, method string) map[string]any {
	t.Helper()
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		for _, msg := range sink.snapshot() {
			if msg["method"] == method {
				return msg
			}
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("missing method %s in %#v", method, sink.snapshot())
	return nil
}

func waitForPermissionResponse(ctx context.Context, bus *eventbus.Bus, correlationID string) bool {
	responseCtx, cancel := context.WithTimeout(ctx, 2*time.Second)
	defer cancel()
	_, err := bus.Await(responseCtx, eventbus.Filter{
		CorrelationID: correlationID,
		Kinds:         []eventbus.Kind{eventbus.KindPermissionResponded},
	})
	return err == nil
}

func hasSessionUpdate(t *testing.T, sink *messageSink, updateKind string) bool {
	t.Helper()
	for _, msg := range sink.snapshot() {
		if msg["method"] != "session/update" {
			continue
		}
		update := objectField(t, objectField(t, msg, "params"), "update")
		if update["sessionUpdate"] == updateKind {
			return true
		}
	}
	return false
}

func assertResponseID(t *testing.T, msg map[string]any, id any) {
	t.Helper()
	if !idMatches(msg["id"], id) {
		t.Fatalf("id = %#v, want %#v in %#v", msg["id"], id, msg)
	}
	if msg["error"] != nil {
		t.Fatalf("response error: %#v", msg["error"])
	}
}

func idMatches(got any, want any) bool {
	switch w := want.(type) {
	case float64:
		return got == w
	case string:
		return got == w
	default:
		return got == want
	}
}

func objectField(t *testing.T, object map[string]any, field string) map[string]any {
	t.Helper()
	value, ok := object[field].(map[string]any)
	if !ok {
		t.Fatalf("%s = %#v", field, object[field])
	}
	return value
}

func sliceField(t *testing.T, object map[string]any, field string) []any {
	t.Helper()
	value, ok := object[field].([]any)
	if !ok {
		t.Fatalf("%s = %#v", field, object[field])
	}
	return value
}

func stringField(t *testing.T, object map[string]any, field string) string {
	t.Helper()
	value, ok := object[field].(string)
	if !ok {
		t.Fatalf("%s = %#v", field, object[field])
	}
	return value
}
