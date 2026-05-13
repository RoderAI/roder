package mcp

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"strings"
	"sync"

	"github.com/modelcontextprotocol/go-sdk/mcp"
	"github.com/pandelisz/gode/internal/godex/eventbus"
)

type ServerConfig struct {
	Command  string
	Args     []string
	Env      map[string]string
	Disabled bool
}

type State string

const (
	StateDisabled  State = "disabled"
	StateStarting  State = "starting"
	StateConnected State = "connected"
	StateError     State = "error"
)

type Tool struct {
	Server      string
	Name        string
	Description string
	InputSchema map[string]any
}

type server struct {
	cfg     ServerConfig
	session *mcp.ClientSession
	tools   []Tool
	state   State
	err     error
}

type Manager struct {
	bus     *eventbus.Bus
	servers map[string]*server
	mu      sync.RWMutex
}

func NewManager(bus *eventbus.Bus, configs map[string]any) *Manager {
	servers := make(map[string]*server)
	for name, raw := range configs {
		var cfg ServerConfig
		data, _ := json.Marshal(raw)
		_ = json.Unmarshal(data, &cfg)
		servers[name] = &server{cfg: cfg, state: StateDisabled}
	}
	return &Manager{bus: bus, servers: servers}
}

func (m *Manager) Start(ctx context.Context) error {
	for name := range m.servers {
		if err := m.StartServer(ctx, name); err != nil {
			return err
		}
	}
	return nil
}

func (m *Manager) StartServer(ctx context.Context, name string) error {
	m.mu.Lock()
	srv, ok := m.servers[name]
	m.mu.Unlock()
	if !ok {
		return fmt.Errorf("mcp server %q not configured", name)
	}
	if srv.cfg.Disabled {
		m.setState(ctx, name, StateDisabled, nil)
		return nil
	}
	if srv.cfg.Command == "" {
		m.setState(ctx, name, StateError, fmt.Errorf("missing command"))
		return fmt.Errorf("mcp server %q missing command", name)
	}

	m.setState(ctx, name, StateStarting, nil)
	cmd := exec.CommandContext(ctx, srv.cfg.Command, srv.cfg.Args...)
	cmd.Env = os.Environ()
	for key, value := range srv.cfg.Env {
		cmd.Env = append(cmd.Env, key+"="+value)
	}
	client := mcp.NewClient(&mcp.Implementation{Name: "gode", Version: "dev"}, nil)
	session, err := client.Connect(ctx, &mcp.CommandTransport{Command: cmd}, nil)
	if err != nil {
		m.setState(ctx, name, StateError, err)
		return err
	}
	list, err := session.ListTools(ctx, &mcp.ListToolsParams{})
	if err != nil {
		_ = session.Close()
		m.setState(ctx, name, StateError, err)
		return err
	}
	tools := make([]Tool, 0, len(list.Tools))
	for _, remoteTool := range list.Tools {
		schema := map[string]any{}
		if remoteTool.InputSchema != nil {
			data, _ := json.Marshal(remoteTool.InputSchema)
			_ = json.Unmarshal(data, &schema)
		}
		tools = append(tools, Tool{
			Server:      name,
			Name:        remoteTool.Name,
			Description: remoteTool.Description,
			InputSchema: schema,
		})
	}

	m.mu.Lock()
	srv.session = session
	srv.tools = tools
	srv.state = StateConnected
	srv.err = nil
	m.mu.Unlock()
	m.publish(ctx, name, StateConnected, nil, map[string]any{"tools": len(tools)})
	return nil
}

func (m *Manager) Tools() []Tool {
	m.mu.RLock()
	defer m.mu.RUnlock()
	var out []Tool
	for _, srv := range m.servers {
		out = append(out, srv.tools...)
	}
	return out
}

func (m *Manager) CallTool(ctx context.Context, serverName, toolName string, input map[string]any) (string, error) {
	m.mu.RLock()
	srv, ok := m.servers[serverName]
	session := (*mcp.ClientSession)(nil)
	if ok {
		session = srv.session
	}
	m.mu.RUnlock()
	if !ok || session == nil {
		return "", fmt.Errorf("mcp server %q is not connected", serverName)
	}
	result, err := session.CallTool(ctx, &mcp.CallToolParams{Name: toolName, Arguments: input})
	if err != nil {
		return "", err
	}
	var parts []string
	for _, content := range result.Content {
		if text, ok := content.(*mcp.TextContent); ok {
			parts = append(parts, text.Text)
		}
	}
	if len(parts) == 0 && result.StructuredContent != nil {
		data, _ := json.MarshalIndent(result.StructuredContent, "", "  ")
		parts = append(parts, string(data))
	}
	return strings.Join(parts, "\n"), result.GetError()
}

func (m *Manager) Close(ctx context.Context) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	for name, srv := range m.servers {
		if srv.session != nil {
			_ = srv.session.Close()
			srv.session = nil
			m.publish(ctx, name, StateDisabled, nil, nil)
		}
	}
	return nil
}

func (m *Manager) setState(ctx context.Context, name string, state State, err error) {
	m.mu.Lock()
	if srv, ok := m.servers[name]; ok {
		srv.state = state
		srv.err = err
	}
	m.mu.Unlock()
	m.publish(ctx, name, state, err, nil)
}

func (m *Manager) publish(ctx context.Context, name string, state State, err error, extra map[string]any) {
	if m.bus == nil {
		return
	}
	payload := map[string]any{"server": name, "state": state}
	if err != nil {
		payload["error"] = err.Error()
	}
	for key, value := range extra {
		payload[key] = value
	}
	m.bus.Publish(ctx, eventbus.Event{Source: eventbus.SourceMCP, Kind: eventbus.KindMCPStateChanged, Payload: payload})
}
