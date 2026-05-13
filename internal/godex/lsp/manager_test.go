package lsp

import (
	"bufio"
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/pandelisz/gode/internal/godex/eventbus"
)

func TestManagerStartsFakeLSPReportsDiagnosticsRestartsAndCloses(t *testing.T) {
	workspace := t.TempDir()
	if err := os.WriteFile(filepath.Join(workspace, "go.mod"), []byte("module test\n"), 0o644); err != nil {
		t.Fatalf("write marker: %v", err)
	}
	target := filepath.Join(workspace, "main.go")
	if err := os.WriteFile(target, []byte("package main\n"), 0o644); err != nil {
		t.Fatalf("write file: %v", err)
	}
	bus := eventbus.New(eventbus.WithSubscriberBuffer(32))
	defer bus.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	events := bus.Subscribe(ctx, eventbus.Filter{})
	manager := NewManager(bus, workspace, map[string]Config{"gopls": fakeConfig(target)})

	if err := manager.Start(ctx); err != nil {
		t.Fatalf("start: %v", err)
	}
	assertLSPState(t, events, "gopls", StateStarting)
	assertLSPState(t, events, "gopls", StateConnected)
	assertLSPDiagnostics(t, events, "gopls", target)
	diagnostics, err := manager.Diagnostics(ctx, target)
	if err != nil {
		t.Fatalf("diagnostics: %v", err)
	}
	if len(diagnostics) != 1 || diagnostics[0].Message != "fake diagnostic" || diagnostics[0].Server != "gopls" {
		t.Fatalf("diagnostics = %#v", diagnostics)
	}
	if err := manager.Restart(ctx, "gopls"); err != nil {
		t.Fatalf("restart: %v", err)
	}
	assertLSPState(t, events, "gopls", StateStarting)
	assertLSPState(t, events, "gopls", StateConnected)
	if err := manager.Close(ctx); err != nil {
		t.Fatalf("close: %v", err)
	}
	assertLSPState(t, events, "gopls", StateClosed)
}

func TestManagerDisabledWhenRootMarkerMissing(t *testing.T) {
	workspace := t.TempDir()
	bus := eventbus.New(eventbus.WithSubscriberBuffer(8))
	defer bus.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	events := bus.Subscribe(ctx, eventbus.Filter{})
	manager := NewManager(bus, workspace, map[string]Config{"gopls": {
		Command:     os.Args[0],
		Args:        []string{"-test.run=TestLSPHelperProcess", "--"},
		Env:         map[string]string{"GODE_LSP_HELPER": "1", "GODE_LSP_DIAG_PATH": filepath.Join(workspace, "main.go")},
		RootMarkers: []string{"go.mod"},
	}})
	if err := manager.Start(ctx); err != nil {
		t.Fatalf("start: %v", err)
	}
	assertLSPState(t, events, "gopls", StateDisabled)
}

func TestLSPHelperProcess(t *testing.T) {
	if os.Getenv("GODE_LSP_HELPER") != "1" {
		return
	}
	reader := bufio.NewReader(os.Stdin)
	for {
		data, err := readMessage(reader)
		if err != nil {
			os.Exit(0)
		}
		var msg struct {
			ID     any    `json:"id"`
			Method string `json:"method"`
		}
		_ = json.Unmarshal(data, &msg)
		if msg.Method != "initialize" {
			continue
		}
		_ = writeMessage(os.Stdout, map[string]any{"jsonrpc": "2.0", "id": msg.ID, "result": map[string]any{"capabilities": map[string]any{}}})
		_ = writeMessage(os.Stdout, map[string]any{
			"jsonrpc": "2.0",
			"method":  "textDocument/publishDiagnostics",
			"params": map[string]any{
				"uri": pathToURI(os.Getenv("GODE_LSP_DIAG_PATH")),
				"diagnostics": []map[string]any{{
					"range": map[string]any{
						"start": map[string]any{"line": 0, "character": 0},
						"end":   map[string]any{"line": 0, "character": 7},
					},
					"severity": 1,
					"message":  "fake diagnostic",
				}},
			},
		})
	}
}

func fakeConfig(target string) Config {
	return Config{
		Command:     os.Args[0],
		Args:        []string{"-test.run=TestLSPHelperProcess", "--"},
		Env:         map[string]string{"GODE_LSP_HELPER": "1", "GODE_LSP_DIAG_PATH": target},
		FileTypes:   []string{".go"},
		RootMarkers: []string{"go.mod"},
	}
}

func assertLSPState(t *testing.T, events <-chan eventbus.Event, server string, state State) {
	t.Helper()
	deadline := time.After(3 * time.Second)
	for {
		select {
		case ev := <-events:
			if ev.Kind != eventbus.KindLSPStateChanged {
				continue
			}
			var payload struct {
				Server string `json:"server"`
				State  State  `json:"state"`
			}
			_ = ev.DecodePayload(&payload)
			if payload.Server == server && payload.State == state {
				return
			}
		case <-deadline:
			t.Fatalf("missing lsp state %s/%s", server, state)
		}
	}
}

func assertLSPDiagnostics(t *testing.T, events <-chan eventbus.Event, server string, path string) {
	t.Helper()
	deadline := time.After(3 * time.Second)
	for {
		select {
		case ev := <-events:
			if ev.Kind != eventbus.KindLSPDiagnostics {
				continue
			}
			var payload struct {
				Server string `json:"server"`
				Path   string `json:"path"`
			}
			_ = ev.DecodePayload(&payload)
			if payload.Server == server && payload.Path == path {
				return
			}
		case <-deadline:
			t.Fatalf("missing lsp diagnostics %s/%s", server, path)
		}
	}
}
