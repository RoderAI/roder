package builtin

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	godexlsp "github.com/pandelisz/gode/internal/godex/lsp"
	"github.com/pandelisz/gode/internal/godex/tools"
)

func TestLSPToolsReturnDiagnosticsAndRestart(t *testing.T) {
	workspace := t.TempDir()
	target := filepath.Join(workspace, "main.go")
	if err := os.WriteFile(target, []byte("package main\n"), 0o644); err != nil {
		t.Fatalf("write file: %v", err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	manager := godexlsp.NewManager(nil, workspace, map[string]godexlsp.Config{"fake": {
		Command: os.Args[0],
		Args:    []string{"-test.run=TestLSPBuiltinHelperProcess", "--"},
		Env:     map[string]string{"GODE_LSP_BUILTIN_HELPER": "1", "GODE_LSP_BUILTIN_DIAG_PATH": target},
	}})
	if err := manager.Start(ctx); err != nil {
		t.Fatalf("start: %v", err)
	}
	defer manager.Close(context.Background())
	waitForLSPDiagnostic(t, ctx, manager, target)

	reg := tools.NewRegistry(tools.WithAutoApprove(true))
	RegisterLSP(reg, manager)
	result, err := reg.Run(ctx, tools.Call{Name: "lsp_diagnostics", Input: map[string]any{"path": target}})
	if err != nil {
		t.Fatalf("diagnostics: %v", err)
	}
	for _, want := range []string{"fake", target, "fake diagnostic", `"severity": 1`} {
		if !strings.Contains(result.Text, want) {
			t.Fatalf("diagnostics missing %q:\n%s", want, result.Text)
		}
	}
	restart, err := reg.Run(ctx, tools.Call{Name: "lsp_restart", Input: map[string]any{"server": "fake"}})
	if err != nil {
		t.Fatalf("restart: %v", err)
	}
	if restart.Text != "restarted fake" {
		t.Fatalf("restart = %q", restart.Text)
	}
}

func TestLSPBuiltinHelperProcess(t *testing.T) {
	if os.Getenv("GODE_LSP_BUILTIN_HELPER") != "1" {
		return
	}
	reader := bufio.NewReader(os.Stdin)
	for {
		data, err := readLSPMessage(reader)
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
		_ = writeLSPMessage(os.Stdout, map[string]any{"jsonrpc": "2.0", "id": msg.ID, "result": map[string]any{"capabilities": map[string]any{}}})
		_ = writeLSPMessage(os.Stdout, map[string]any{
			"jsonrpc": "2.0",
			"method":  "textDocument/publishDiagnostics",
			"params": map[string]any{
				"uri": pathToFileURI(os.Getenv("GODE_LSP_BUILTIN_DIAG_PATH")),
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

func waitForLSPDiagnostic(t *testing.T, ctx context.Context, manager *godexlsp.Manager, path string) {
	t.Helper()
	deadline := time.Now().Add(3 * time.Second)
	for time.Now().Before(deadline) {
		diagnostics, err := manager.Diagnostics(ctx, path)
		if err != nil {
			t.Fatalf("diagnostics: %v", err)
		}
		if len(diagnostics) > 0 {
			return
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("timed out waiting for diagnostics")
}

func readLSPMessage(reader *bufio.Reader) ([]byte, error) {
	contentLength := 0
	for {
		line, err := reader.ReadString('\n')
		if err != nil {
			return nil, err
		}
		line = strings.TrimRight(line, "\r\n")
		if line == "" {
			break
		}
		key, value, ok := strings.Cut(line, ":")
		if ok && strings.EqualFold(strings.TrimSpace(key), "Content-Length") {
			_, _ = fmt.Sscanf(strings.TrimSpace(value), "%d", &contentLength)
		}
	}
	data := make([]byte, contentLength)
	_, err := io.ReadFull(reader, data)
	return data, err
}

func writeLSPMessage(writer io.Writer, message map[string]any) error {
	data, err := json.Marshal(message)
	if err != nil {
		return err
	}
	_, err = fmt.Fprintf(writer, "Content-Length: %d\r\n\r\n%s", len(data), data)
	return err
}

func pathToFileURI(path string) string {
	return "file://" + filepath.ToSlash(path)
}
