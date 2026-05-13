package mcp

import (
	"context"
	"os"
	"reflect"
	"strings"
	"testing"
	"time"

	sdkmcp "github.com/modelcontextprotocol/go-sdk/mcp"
	"github.com/pandelisz/gode/internal/godex/eventbus"
)

func TestConfigParsesLegacyRawMap(t *testing.T) {
	configs, err := ParseConfigMap(map[string]any{
		"dev": map[string]any{
			"command":        "/bin/echo",
			"args":           []any{"hello"},
			"enabled_tools":  []any{"echo"},
			"disabled_tools": []any{"danger"},
			"timeout":        float64(3),
		},
	})
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	cfg := configs["dev"]
	if cfg.Type != "stdio" || cfg.Command != "/bin/echo" || cfg.Timeout != 3 {
		t.Fatalf("config = %#v", cfg)
	}
	if !reflect.DeepEqual(cfg.EnabledTools, []string{"echo"}) || !reflect.DeepEqual(cfg.DisabledTools, []string{"danger"}) {
		t.Fatalf("tool filters = %#v %#v", cfg.EnabledTools, cfg.DisabledTools)
	}
}

func TestManagerDisabledServerPublishesStateAndDoesNotStart(t *testing.T) {
	bus := eventbus.New(eventbus.WithSubscriberBuffer(8))
	defer bus.Close()
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	events := bus.Subscribe(ctx, eventbus.Filter{Kinds: []eventbus.Kind{eventbus.KindMCPStateChanged}})

	manager := NewManager(bus, map[string]ServerConfig{"disabled": {Disabled: true}})
	if err := manager.Start(ctx); err != nil {
		t.Fatalf("start: %v", err)
	}
	if len(manager.Tools()) != 0 {
		t.Fatalf("tools = %#v", manager.Tools())
	}
	assertMCPState(t, events, "disabled", StateDisabled)
}

func TestManagerStartsStdioServerAndFiltersTools(t *testing.T) {
	bus := eventbus.New(eventbus.WithSubscriberBuffer(16))
	defer bus.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	events := bus.Subscribe(ctx, eventbus.Filter{Kinds: []eventbus.Kind{eventbus.KindMCPStateChanged}})

	manager := NewManager(bus, map[string]ServerConfig{
		"helper": helperConfig([]string{"echo"}),
	})
	if err := manager.Start(ctx); err != nil {
		t.Fatalf("start: %v", err)
	}
	defer manager.Close(context.Background())

	tools := manager.Tools()
	if len(tools) != 1 || tools[0].Name != "echo" || tools[0].Server != "helper" {
		t.Fatalf("tools = %#v", tools)
	}
	assertMCPState(t, events, "helper", StateStarting)
	assertMCPState(t, events, "helper", StateConnected)
}

func TestManagerWithNoEnabledToolsRegistersNone(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	manager := NewManager(nil, map[string]ServerConfig{
		"helper": helperConfig([]string{"missing"}),
	})
	if err := manager.Start(ctx); err != nil {
		t.Fatalf("start: %v", err)
	}
	defer manager.Close(context.Background())
	if len(manager.Tools()) != 0 {
		t.Fatalf("tools = %#v", manager.Tools())
	}
}

func TestManagerTransportSupportAndUnsupportedTransportError(t *testing.T) {
	for _, cfg := range []ServerConfig{
		{Type: "http", URL: "http://127.0.0.1/mcp"},
		{Type: "streamable_http", URL: "http://127.0.0.1/mcp"},
		{Type: "sse", URL: "http://127.0.0.1/sse"},
	} {
		if _, err := (&server{cfg: cfg.withDefaults()}).transport(context.Background()); err != nil {
			t.Fatalf("transport %s: %v", cfg.Type, err)
		}
	}
	manager := NewManager(nil, map[string]ServerConfig{"bad": {Type: "websocket", URL: "ws://127.0.0.1"}})
	if err := manager.Start(context.Background()); err == nil || !strings.Contains(err.Error(), `unsupported transport "websocket"`) {
		t.Fatalf("err = %v", err)
	}
}

func TestManagerClosePublishesClosedState(t *testing.T) {
	bus := eventbus.New(eventbus.WithSubscriberBuffer(16))
	defer bus.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	events := bus.Subscribe(ctx, eventbus.Filter{Kinds: []eventbus.Kind{eventbus.KindMCPStateChanged}})
	manager := NewManager(bus, map[string]ServerConfig{"helper": helperConfig(nil)})
	if err := manager.Start(ctx); err != nil {
		t.Fatalf("start: %v", err)
	}
	if err := manager.Close(ctx); err != nil {
		t.Fatalf("close: %v", err)
	}
	assertMCPState(t, events, "helper", StateStarting)
	assertMCPState(t, events, "helper", StateConnected)
	assertMCPState(t, events, "helper", StateClosed)
}

func TestMCPHelperProcess(t *testing.T) {
	if os.Getenv("GODE_MCP_HELPER") != "1" {
		return
	}
	server := sdkmcp.NewServer(&sdkmcp.Implementation{Name: "helper", Version: "test"}, nil)
	type echoArgs struct {
		Text string `json:"text"`
	}
	sdkmcp.AddTool(server, &sdkmcp.Tool{Name: "echo", Description: "echo text"}, func(_ context.Context, _ *sdkmcp.CallToolRequest, args echoArgs) (*sdkmcp.CallToolResult, any, error) {
		return &sdkmcp.CallToolResult{Content: []sdkmcp.Content{&sdkmcp.TextContent{Text: args.Text}}}, nil, nil
	})
	sdkmcp.AddTool(server, &sdkmcp.Tool{Name: "danger", Description: "danger"}, func(_ context.Context, _ *sdkmcp.CallToolRequest, args echoArgs) (*sdkmcp.CallToolResult, any, error) {
		return &sdkmcp.CallToolResult{Content: []sdkmcp.Content{&sdkmcp.TextContent{Text: "danger"}}}, nil, nil
	})
	if err := server.Run(context.Background(), &sdkmcp.StdioTransport{}); err != nil {
		os.Exit(1)
	}
	os.Exit(0)
}

func helperConfig(enabled []string) ServerConfig {
	return ServerConfig{
		Type:         "stdio",
		Command:      os.Args[0],
		Args:         []string{"-test.run=TestMCPHelperProcess", "--"},
		Env:          map[string]string{"GODE_MCP_HELPER": "1"},
		EnabledTools: enabled,
		Timeout:      5,
	}
}

func assertMCPState(t *testing.T, events <-chan eventbus.Event, server string, state State) {
	t.Helper()
	deadline := time.After(3 * time.Second)
	for {
		select {
		case ev := <-events:
			var payload struct {
				Server string `json:"server"`
				State  State  `json:"state"`
			}
			_ = ev.DecodePayload(&payload)
			if payload.Server == server && payload.State == state {
				return
			}
		case <-deadline:
			t.Fatalf("missing mcp state %s/%s", server, state)
		}
	}
}
