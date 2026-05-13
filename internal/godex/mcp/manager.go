package mcp

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"os/exec"
	"strings"
	"sync"

	sdkmcp "github.com/modelcontextprotocol/go-sdk/mcp"
	"github.com/pandelisz/gode/internal/godex/eventbus"
)

type State string

const (
	StateDisabled  State = "disabled"
	StateStarting  State = "starting"
	StateConnected State = "connected"
	StateError     State = "error"
	StateClosed    State = "closed"
)

type Tool struct {
	Server      string
	Name        string
	Description string
	InputSchema map[string]any
}

type ServerState struct {
	Server    string `json:"server"`
	State     State  `json:"state"`
	Error     string `json:"error,omitempty"`
	Tools     int    `json:"tools,omitempty"`
	Resources int    `json:"resources,omitempty"`
	Prompts   int    `json:"prompts,omitempty"`
}

type server struct {
	cfg       ServerConfig
	session   *sdkmcp.ClientSession
	tools     []Tool
	resources []Resource
	prompts   []Prompt
	state     State
	err       error
}

type Manager struct {
	bus     *eventbus.Bus
	servers map[string]*server
	mu      sync.RWMutex
}

func NewManager(bus *eventbus.Bus, configs map[string]ServerConfig) *Manager {
	servers := make(map[string]*server)
	for name, raw := range configs {
		cfg := raw.withDefaults()
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

func (m *Manager) AddStdioServer(ctx context.Context, name string, cfg ServerConfig) error {
	if strings.TrimSpace(name) == "" {
		return fmt.Errorf("mcp server name is required")
	}
	cfg = cfg.withDefaults()
	m.mu.Lock()
	if existing := m.servers[name]; existing != nil && existing.session != nil {
		_ = existing.session.Close()
	}
	m.servers[name] = &server{cfg: cfg, state: StateDisabled}
	m.mu.Unlock()
	return m.StartServer(ctx, name)
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
	m.setState(ctx, name, StateStarting, nil)
	transport, err := srv.transport(ctx)
	if err != nil {
		m.setState(ctx, name, StateError, err)
		return err
	}
	client := sdkmcp.NewClient(&sdkmcp.Implementation{Name: "gode", Version: "dev"}, nil)
	session, err := client.Connect(ctx, transport, nil)
	if err != nil {
		m.setState(ctx, name, StateError, err)
		return err
	}
	list, err := session.ListTools(ctx, &sdkmcp.ListToolsParams{})
	if err != nil {
		_ = session.Close()
		m.setState(ctx, name, StateError, err)
		return err
	}
	tools := make([]Tool, 0, len(list.Tools))
	for _, remoteTool := range list.Tools {
		if !srv.cfg.toolEnabled(remoteTool.Name) {
			continue
		}
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
	resources, _ := sessionResources(ctx, name, session)
	prompts, _ := sessionPrompts(ctx, name, session)

	m.mu.Lock()
	srv.session = session
	srv.tools = tools
	srv.resources = resources
	srv.prompts = prompts
	srv.state = StateConnected
	srv.err = nil
	m.mu.Unlock()
	m.publish(ctx, name, StateConnected, nil, map[string]any{"tools": len(tools), "resources": len(resources), "prompts": len(prompts)})
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

func (m *Manager) States() []ServerState {
	m.mu.RLock()
	defer m.mu.RUnlock()
	out := make([]ServerState, 0, len(m.servers))
	for name, srv := range m.servers {
		state := ServerState{
			Server:    name,
			State:     srv.state,
			Tools:     len(srv.tools),
			Resources: len(srv.resources),
			Prompts:   len(srv.prompts),
		}
		if srv.err != nil {
			state.Error = srv.err.Error()
		}
		out = append(out, state)
	}
	return out
}

func (m *Manager) Resources() []Resource {
	m.mu.RLock()
	defer m.mu.RUnlock()
	var out []Resource
	for _, srv := range m.servers {
		out = append(out, srv.resources...)
	}
	return out
}

func (m *Manager) Prompts() []Prompt {
	m.mu.RLock()
	defer m.mu.RUnlock()
	var out []Prompt
	for _, srv := range m.servers {
		out = append(out, srv.prompts...)
	}
	return out
}

func (m *Manager) ReadResource(ctx context.Context, serverName string, uri string) (string, error) {
	m.mu.RLock()
	srv, ok := m.servers[serverName]
	session := (*sdkmcp.ClientSession)(nil)
	if ok {
		session = srv.session
	}
	m.mu.RUnlock()
	if !ok || session == nil {
		return "", fmt.Errorf("mcp server %q is not connected", serverName)
	}
	result, err := session.ReadResource(ctx, &sdkmcp.ReadResourceParams{URI: uri})
	if err != nil {
		return "", err
	}
	return formatResourceContents(result.Contents), nil
}

func (m *Manager) GetPrompt(ctx context.Context, serverName string, name string, args map[string]string) (string, error) {
	m.mu.RLock()
	srv, ok := m.servers[serverName]
	session := (*sdkmcp.ClientSession)(nil)
	if ok {
		session = srv.session
	}
	m.mu.RUnlock()
	if !ok || session == nil {
		return "", fmt.Errorf("mcp server %q is not connected", serverName)
	}
	result, err := session.GetPrompt(ctx, &sdkmcp.GetPromptParams{Name: name, Arguments: args})
	if err != nil {
		return "", err
	}
	return formatPromptMessages(result), nil
}

func (m *Manager) CallTool(ctx context.Context, serverName, toolName string, input map[string]any) (string, error) {
	m.mu.RLock()
	srv, ok := m.servers[serverName]
	session := (*sdkmcp.ClientSession)(nil)
	if ok {
		session = srv.session
	}
	m.mu.RUnlock()
	if !ok || session == nil {
		return "", fmt.Errorf("mcp server %q is not connected", serverName)
	}
	result, err := session.CallTool(ctx, &sdkmcp.CallToolParams{Name: toolName, Arguments: input})
	if err != nil {
		return "", err
	}
	var parts []string
	for _, content := range result.Content {
		if text, ok := content.(*sdkmcp.TextContent); ok {
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
			srv.state = StateClosed
			m.publish(ctx, name, StateClosed, nil, nil)
		}
	}
	return nil
}

func sessionResources(ctx context.Context, serverName string, session *sdkmcp.ClientSession) ([]Resource, error) {
	list, err := session.ListResources(ctx, &sdkmcp.ListResourcesParams{})
	if err != nil {
		return nil, err
	}
	out := make([]Resource, 0, len(list.Resources))
	for _, item := range list.Resources {
		out = append(out, Resource{
			Server:      serverName,
			URI:         item.URI,
			Name:        item.Name,
			Title:       item.Title,
			Description: item.Description,
			MIMEType:    item.MIMEType,
			Size:        item.Size,
		})
	}
	return out, nil
}

func sessionPrompts(ctx context.Context, serverName string, session *sdkmcp.ClientSession) ([]Prompt, error) {
	list, err := session.ListPrompts(ctx, &sdkmcp.ListPromptsParams{})
	if err != nil {
		return nil, err
	}
	out := make([]Prompt, 0, len(list.Prompts))
	for _, item := range list.Prompts {
		out = append(out, Prompt{
			Server:      serverName,
			Name:        item.Name,
			Title:       item.Title,
			Description: item.Description,
			Arguments:   promptArguments(item.Arguments),
		})
	}
	return out, nil
}

func promptArguments(args []*sdkmcp.PromptArgument) []PromptArgument {
	out := make([]PromptArgument, 0, len(args))
	for _, arg := range args {
		out = append(out, PromptArgument{Name: arg.Name, Description: arg.Description, Required: arg.Required})
	}
	return out
}

func formatResourceContents(contents []*sdkmcp.ResourceContents) string {
	var parts []string
	for _, content := range contents {
		switch {
		case content.Text != "":
			parts = append(parts, content.Text)
		case len(content.Blob) > 0:
			parts = append(parts, base64.StdEncoding.EncodeToString(content.Blob))
		default:
			data, _ := json.MarshalIndent(content, "", "  ")
			parts = append(parts, string(data))
		}
	}
	return strings.Join(parts, "\n")
}

func formatPromptMessages(result *sdkmcp.GetPromptResult) string {
	var buf bytes.Buffer
	if result.Description != "" {
		buf.WriteString(result.Description)
		buf.WriteString("\n\n")
	}
	for i, message := range result.Messages {
		if i > 0 {
			buf.WriteString("\n\n")
		}
		buf.WriteString(string(message.Role))
		buf.WriteString(": ")
		if text, ok := message.Content.(*sdkmcp.TextContent); ok {
			buf.WriteString(text.Text)
			continue
		}
		data, _ := json.Marshal(message.Content)
		buf.Write(data)
	}
	return strings.TrimSpace(buf.String())
}

func (s *server) transport(ctx context.Context) (sdkmcp.Transport, error) {
	switch s.cfg.Type {
	case "", "stdio":
		if strings.TrimSpace(s.cfg.Command) == "" {
			return nil, fmt.Errorf("missing command")
		}
		cmd := exec.CommandContext(ctx, s.cfg.Command, s.cfg.Args...)
		cmd.Env = os.Environ()
		for key, value := range s.cfg.Env {
			cmd.Env = append(cmd.Env, key+"="+value)
		}
		return &sdkmcp.CommandTransport{Command: cmd}, nil
	case "http", "streamable_http":
		if strings.TrimSpace(s.cfg.URL) == "" {
			return nil, fmt.Errorf("missing url")
		}
		return &sdkmcp.StreamableClientTransport{Endpoint: s.cfg.URL, HTTPClient: headerClient(s.cfg.Headers)}, nil
	case "sse":
		if strings.TrimSpace(s.cfg.URL) == "" {
			return nil, fmt.Errorf("missing url")
		}
		return &sdkmcp.SSEClientTransport{Endpoint: s.cfg.URL, HTTPClient: headerClient(s.cfg.Headers)}, nil
	default:
		return nil, fmt.Errorf("unsupported transport %q", s.cfg.Type)
	}
}

func (c ServerConfig) toolEnabled(name string) bool {
	enabled := stringSet(c.EnabledTools)
	disabled := stringSet(c.DisabledTools)
	if _, ok := disabled[name]; ok {
		return false
	}
	if len(enabled) == 0 {
		return true
	}
	_, ok := enabled[name]
	return ok
}

func stringSet(values []string) map[string]struct{} {
	out := map[string]struct{}{}
	for _, value := range values {
		if trimmed := strings.TrimSpace(value); trimmed != "" {
			out[trimmed] = struct{}{}
		}
	}
	return out
}

func headerClient(headers map[string]string) *http.Client {
	if len(headers) == 0 {
		return nil
	}
	return &http.Client{Transport: headerRoundTripper{headers: headers, next: http.DefaultTransport}}
}

type headerRoundTripper struct {
	headers map[string]string
	next    http.RoundTripper
}

func (r headerRoundTripper) RoundTrip(req *http.Request) (*http.Response, error) {
	clone := req.Clone(req.Context())
	for key, value := range r.headers {
		clone.Header.Set(key, value)
	}
	return r.next.RoundTrip(clone)
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
