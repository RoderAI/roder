package lsp

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"

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

type server struct {
	cfg    Config
	state  State
	err    error
	client *client
}

type Manager struct {
	bus       *eventbus.Bus
	workspace string
	mu        sync.RWMutex
	servers   map[string]*server
	diags     map[string][]Diagnostic
}

func NewManager(bus *eventbus.Bus, workspace string, configs map[string]Config) *Manager {
	servers := make(map[string]*server, len(configs))
	for name, cfg := range configs {
		servers[name] = &server{cfg: cfg, state: StateDisabled}
	}
	return &Manager{bus: bus, workspace: workspace, servers: servers, diags: map[string][]Diagnostic{}}
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
	m.mu.RLock()
	srv, ok := m.servers[name]
	m.mu.RUnlock()
	if !ok {
		return fmt.Errorf("lsp server %q not configured", name)
	}
	if srv.cfg.Disabled || !m.rootMarkersMatch(srv.cfg.RootMarkers) {
		m.setState(ctx, name, StateDisabled, nil)
		return nil
	}
	m.setState(ctx, name, StateStarting, nil)
	c, err := startClient(ctx, name, m.workspace, srv.cfg, func(path string, diagnostics []Diagnostic) {
		m.setDiagnostics(context.Background(), name, path, diagnostics)
	})
	if err != nil {
		m.setState(ctx, name, StateError, err)
		return err
	}
	m.mu.Lock()
	srv.client = c
	srv.state = StateConnected
	srv.err = nil
	m.mu.Unlock()
	m.publish(ctx, name, StateConnected, nil)
	return nil
}

func (m *Manager) Diagnostics(ctx context.Context, path string) ([]Diagnostic, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	abs, err := filepath.Abs(path)
	if err != nil {
		return nil, err
	}
	m.mu.RLock()
	defer m.mu.RUnlock()
	return append([]Diagnostic(nil), m.diags[abs]...), nil
}

func (m *Manager) Restart(ctx context.Context, name string) error {
	m.mu.Lock()
	srv, ok := m.servers[name]
	if !ok {
		m.mu.Unlock()
		return fmt.Errorf("lsp server %q not configured", name)
	}
	if srv.client != nil {
		_ = srv.client.close()
		srv.client = nil
	}
	m.mu.Unlock()
	return m.StartServer(ctx, name)
}

func (m *Manager) Close(ctx context.Context) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	for name, srv := range m.servers {
		if srv.client != nil {
			_ = srv.client.close()
			srv.client = nil
			srv.state = StateClosed
			m.publish(ctx, name, StateClosed, nil)
		}
	}
	return nil
}

func (m *Manager) rootMarkersMatch(markers []string) bool {
	if len(markers) == 0 {
		return true
	}
	for _, marker := range markers {
		if strings.TrimSpace(marker) == "" {
			continue
		}
		if _, err := os.Stat(filepath.Join(m.workspace, marker)); err == nil {
			return true
		}
	}
	return false
}

func (m *Manager) setState(ctx context.Context, name string, state State, err error) {
	m.mu.Lock()
	if srv, ok := m.servers[name]; ok {
		srv.state = state
		srv.err = err
	}
	m.mu.Unlock()
	m.publish(ctx, name, state, err)
}

func (m *Manager) setDiagnostics(ctx context.Context, name string, path string, diagnostics []Diagnostic) {
	abs, err := filepath.Abs(path)
	if err != nil {
		abs = path
	}
	for i := range diagnostics {
		diagnostics[i].Server = name
		diagnostics[i].Path = abs
	}
	m.mu.Lock()
	m.diags[abs] = append([]Diagnostic(nil), diagnostics...)
	m.mu.Unlock()
	if m.bus != nil {
		m.bus.Publish(ctx, eventbus.Event{Source: eventbus.SourceLSP, Kind: eventbus.KindLSPDiagnostics, Payload: map[string]any{
			"server":      name,
			"path":        abs,
			"diagnostics": diagnostics,
		}})
	}
}

func (m *Manager) publish(ctx context.Context, name string, state State, err error) {
	if m.bus == nil {
		return
	}
	payload := map[string]any{"server": name, "state": state}
	if err != nil {
		payload["error"] = err.Error()
	}
	m.bus.Publish(ctx, eventbus.Event{Source: eventbus.SourceLSP, Kind: eventbus.KindLSPStateChanged, Payload: payload})
}
