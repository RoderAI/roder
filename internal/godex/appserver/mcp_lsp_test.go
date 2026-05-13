package appserver

import (
	"context"
	"os"
	"path/filepath"
	"testing"
	"time"

	sdkmcp "github.com/modelcontextprotocol/go-sdk/mcp"
	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/lsp"
	"github.com/pandelisz/gode/internal/godex/mcp"
)

func TestMCPLSPHandlersExposeStateResourcesAndDiagnostics(t *testing.T) {
	ctx := context.Background()
	workspace := filepath.Join(t.TempDir(), "workspace")
	app, err := godex.New(ctx, godex.Config{
		Workspace:   workspace,
		DataDir:     t.TempDir(),
		Provider:    "mock",
		AutoApprove: true,
		MCP: map[string]mcp.ServerConfig{
			"disabled-mcp": {Disabled: true},
		},
		LSP: map[string]lsp.Config{
			"disabled-lsp": {Disabled: true},
		},
	})
	if err != nil {
		t.Fatalf("new app: %v", err)
	}
	defer app.Close(ctx)

	var messages []Message
	conn := New(app, Options{Version: "test"}).NewConnection(func(_ context.Context, msg Message) error {
		messages = append(messages, msg)
		return nil
	})
	initializeTestConnection(t, conn)

	sendJSONRequest(t, conn, map[string]any{"id": 2, "method": "mcp/state"})
	mcpServers := sliceField(t, responseResult(t, messages, 2), "servers")
	if len(mcpServers) != 1 || stringField(t, mcpServers[0].(map[string]any), "server") != "disabled-mcp" {
		t.Fatalf("mcp state = %#v", mcpServers)
	}

	sendJSONRequest(t, conn, map[string]any{"id": 3, "method": "mcp/resources/list"})
	if resources := sliceField(t, responseResult(t, messages, 3), "resources"); len(resources) != 0 {
		t.Fatalf("mcp resources = %#v", resources)
	}

	sendJSONRequest(t, conn, map[string]any{"id": 4, "method": "lsp/state"})
	lspServers := sliceField(t, responseResult(t, messages, 4), "servers")
	if len(lspServers) != 1 || stringField(t, lspServers[0].(map[string]any), "server") != "disabled-lsp" {
		t.Fatalf("lsp state = %#v", lspServers)
	}

	sendJSONRequest(t, conn, map[string]any{
		"id":     5,
		"method": "lsp/diagnostics",
		"params": map[string]any{"path": filepath.Join(workspace, "main.go")},
	})
	if diagnostics := sliceField(t, responseResult(t, messages, 5), "diagnostics"); len(diagnostics) != 0 {
		t.Fatalf("diagnostics = %#v", diagnostics)
	}
}

func TestMCPAndLSPNotificationsRespectOptOut(t *testing.T) {
	ctx := context.Background()
	app, err := godex.New(ctx, godex.Config{
		Workspace:   filepath.Join(t.TempDir(), "workspace"),
		DataDir:     t.TempDir(),
		Provider:    "mock",
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("new app: %v", err)
	}
	defer app.Close(ctx)

	var messages []Message
	conn := New(app, Options{Version: "test"}).NewConnection(func(_ context.Context, msg Message) error {
		messages = append(messages, msg)
		return nil
	})
	if err := conn.HandleJSON(ctx, []byte(`{"id":1,"method":"initialize","params":{"clientInfo":{"name":"gode_test"},"capabilities":{"optOutNotificationMethods":["mcp/state/changed"]}}}`)); err != nil {
		t.Fatalf("initialize: %v", err)
	}

	app.Bus.Publish(ctx, eventbus.Event{
		Source: eventbus.SourceMCP,
		Kind:   eventbus.KindMCPStateChanged,
		Payload: map[string]any{
			"server": "docs",
			"state":  "connected",
		},
	})
	app.Bus.Publish(ctx, eventbus.Event{
		Source: eventbus.SourceLSP,
		Kind:   eventbus.KindLSPStateChanged,
		Payload: map[string]any{
			"server": "gopls",
			"state":  "connected",
		},
	})

	waitFor(t, time.Second, func() bool {
		return notificationByMethod(messages, "lsp/state/changed") != nil
	})
	if notificationByMethod(messages, "mcp/state/changed") != nil {
		t.Fatalf("mcp opt-out notification was delivered: %#v", messages)
	}
}

func TestMCPResourceReadHandler(t *testing.T) {
	ctx := context.Background()
	exe, err := os.Executable()
	if err != nil {
		t.Fatalf("executable: %v", err)
	}
	app, err := godex.New(ctx, godex.Config{
		Workspace: filepath.Join(t.TempDir(), "workspace"),
		DataDir:   t.TempDir(),
		Provider:  "mock",
		MCP: map[string]mcp.ServerConfig{
			"helper": {
				Command: exe,
				Args:    []string{"-test.run=TestAppServerMCPHelperProcess", "--"},
				Env:     map[string]string{"GODE_APPSERVER_MCP_HELPER": "1"},
			},
		},
	})
	if err != nil {
		t.Fatalf("new app: %v", err)
	}
	defer app.Close(ctx)

	var messages []Message
	conn := New(app, Options{Version: "test"}).NewConnection(func(_ context.Context, msg Message) error {
		messages = append(messages, msg)
		return nil
	})
	initializeTestConnection(t, conn)

	sendJSONRequest(t, conn, map[string]any{"id": 2, "method": "mcp/resources/list"})
	resources := sliceField(t, responseResult(t, messages, 2), "resources")
	if len(resources) != 1 || stringField(t, resources[0].(map[string]any), "uri") != "file:///notes.txt" {
		t.Fatalf("resources = %#v", resources)
	}

	sendJSONRequest(t, conn, map[string]any{
		"id":     3,
		"method": "mcp/resource/read",
		"params": map[string]any{"server": "helper", "uri": "file:///notes.txt"},
	})
	if got := responseResult(t, messages, 3)["text"]; got != "resource notes" {
		t.Fatalf("resource text = %#v", got)
	}
}

func TestAppServerMCPHelperProcess(t *testing.T) {
	if os.Getenv("GODE_APPSERVER_MCP_HELPER") != "1" {
		return
	}
	server := sdkmcp.NewServer(&sdkmcp.Implementation{Name: "gode-appserver-test-helper", Version: "test"}, nil)
	server.AddResource(&sdkmcp.Resource{URI: "file:///notes.txt", Name: "notes", MIMEType: "text/plain"}, func(_ context.Context, _ *sdkmcp.ReadResourceRequest) (*sdkmcp.ReadResourceResult, error) {
		return &sdkmcp.ReadResourceResult{Contents: []*sdkmcp.ResourceContents{{URI: "file:///notes.txt", MIMEType: "text/plain", Text: "resource notes"}}}, nil
	})
	if err := server.Run(context.Background(), &sdkmcp.StdioTransport{}); err != nil {
		os.Exit(1)
	}
	os.Exit(0)
}
