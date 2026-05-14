package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"path/filepath"
	"testing"
	"time"

	"github.com/pandelisz/gode/internal/godex/appserver"
)

func TestAppServerRemoteParseDefaultsToWebSocket(t *testing.T) {
	root := t.TempDir()
	workspace := filepath.Join(root, "workspace")
	dataDir := filepath.Join(root, "data")
	cfg, listen, err := parseAppServerConfig([]string{
		"--remote",
		"--workspace", workspace,
		"--data-dir", dataDir,
		"--provider", "mock",
		"--model", "mock",
		"--reasoning", "none",
	})
	if err != nil {
		t.Fatalf("parse remote app-server: %v", err)
	}
	if cfg.Workspace != workspace {
		t.Fatalf("workspace = %q", cfg.Workspace)
	}
	if listen.Kind != appserver.TransportWebSocket || listen.Address != "0.0.0.0:0" {
		t.Fatalf("listen = %#v", listen)
	}
	if !listen.Remote.Enabled || !listen.Remote.PrintQR {
		t.Fatalf("remote config = %#v", listen.Remote)
	}
}

func TestAppServerRemoteParseAuthTokenEnv(t *testing.T) {
	t.Setenv("GODE_REMOTE_TOKEN", "env-secret")
	_, listen, err := parseAppServerConfig([]string{
		"--remote",
		"--listen", "ws://127.0.0.1:0",
		"--auth-token", "env:GODE_REMOTE_TOKEN",
		"--print-qr=false",
		"--allowed-origin", "https://phone.example",
		"--allowed-origin", "https://other.example,https://third.example",
		"--provider", "mock",
		"--model", "mock",
		"--reasoning", "none",
	})
	if err != nil {
		t.Fatalf("parse remote app-server: %v", err)
	}
	if listen.Remote.AuthToken != "env-secret" {
		t.Fatalf("auth token = %q", listen.Remote.AuthToken)
	}
	if listen.Remote.PrintQR {
		t.Fatalf("print qr should be false")
	}
	if got := fmt.Sprint(listen.Remote.AllowedOrigins); got != "[https://phone.example https://other.example https://third.example]" {
		t.Fatalf("allowed origins = %s", got)
	}
}

func TestAppServerRemoteRejectsEmptyEnvToken(t *testing.T) {
	t.Setenv("GODE_REMOTE_TOKEN", "")
	_, _, err := parseAppServerConfig([]string{
		"--remote",
		"--auth-token", "env:GODE_REMOTE_TOKEN",
		"--provider", "mock",
		"--model", "mock",
		"--reasoning", "none",
	})
	if err == nil {
		t.Fatal("expected empty env token error")
	}
}

func TestPrintRemoteServerInfoDoesNotPrintFullToken(t *testing.T) {
	auth, err := appserver.NewRemoteAuth("full-secret-token-value", time.Unix(10, 0))
	if err != nil {
		t.Fatalf("new auth: %v", err)
	}
	var stderr bytes.Buffer
	listener := &appserver.WebSocketListener{}
	err = printRemoteServerInfo(&stderr, "gode app-server", "/repo", listener, auth, "full-secret-token-value", false)
	if err != nil {
		t.Fatalf("print remote info: %v", err)
	}
	if bytes.Contains(stderr.Bytes(), []byte("full-secret-token-value")) {
		t.Fatalf("stderr exposed full token:\n%s", stderr.String())
	}
	if !bytes.Contains(stderr.Bytes(), []byte(auth.TokenPreview)) {
		t.Fatalf("stderr missing token preview:\n%s", stderr.String())
	}
}

func TestAppServerServeOffUsesWorkspaceFlag(t *testing.T) {
	root := t.TempDir()
	workspace := filepath.Join(root, "workspace")
	dataDir := filepath.Join(root, "data")
	cfg, listen, err := parseAppServerConfig([]string{
		"--listen", "off",
		"--workspace", workspace,
		"--data-dir", dataDir,
		"--provider", "mock",
		"--model", "mock",
		"--reasoning", "none",
	})
	if err != nil {
		t.Fatalf("parse app-server: %v", err)
	}
	if listen.Kind != appserver.TransportOff {
		t.Fatalf("listen kind = %v", listen.Kind)
	}
	if cfg.Workspace != workspace {
		t.Fatalf("workspace = %q", cfg.Workspace)
	}
	if err := run(context.Background(), []string{
		"app-server",
		"--listen", "off",
		"--workspace", workspace,
		"--data-dir", dataDir,
		"--provider", "mock",
		"--model", "mock",
		"--reasoning", "none",
	}); err != nil {
		t.Fatalf("app-server off: %v", err)
	}
}

func TestAppServerStdioRunsFullPromptWithMockProvider(t *testing.T) {
	root := t.TempDir()
	workspace := filepath.Join(root, "workspace")
	dataDir := filepath.Join(root, "data")
	inputReader, inputWriter := io.Pipe()
	outputReader, outputWriter := io.Pipe()
	var stderr bytes.Buffer
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()

	errCh := make(chan error, 1)
	go func() {
		cfg, listen, err := parseAppServerConfig([]string{
			"--listen", "stdio://",
			"--workspace", workspace,
			"--data-dir", dataDir,
			"--provider", "mock",
			"--model", "mock",
			"--reasoning", "none",
		})
		if err != nil {
			errCh <- err
			return
		}
		errCh <- serveWithConfig(ctx, "gode app-server", cfg, listen, serveIO{stdin: inputReader, stdout: outputWriter, stderr: &stderr})
	}()

	encoder := json.NewEncoder(inputWriter)
	decoder := json.NewDecoder(outputReader)
	mustEncode(t, encoder, map[string]any{
		"id":     1,
		"method": "initialize",
		"params": map[string]any{"clientInfo": map[string]any{"name": "app-server-test"}},
	})
	initMsg := readMessage(t, decoder, func(msg appserver.Message) bool { return messageID(msg.ID) == "1" })
	if initMsg.Error != nil {
		t.Fatalf("initialize error: %#v", initMsg.Error)
	}

	mustEncode(t, encoder, map[string]any{"method": "initialized"})
	mustEncode(t, encoder, map[string]any{
		"id":     2,
		"method": "thread/start",
		"params": map[string]any{"cwd": workspace, "model": "mock"},
	})
	threadMsg := readMessage(t, decoder, func(msg appserver.Message) bool { return messageID(msg.ID) == "2" })
	threadResult := threadMsg.Result.(map[string]any)
	thread := threadResult["thread"].(map[string]any)
	threadID := thread["id"].(string)

	mustEncode(t, encoder, map[string]any{
		"id":     3,
		"method": "turn/start",
		"params": map[string]any{
			"threadId": threadID,
			"input": []map[string]any{
				{"type": "text", "text": "hello from app server"},
			},
		},
	})
	turnMsg := readMessage(t, decoder, func(msg appserver.Message) bool { return messageID(msg.ID) == "3" })
	if turnMsg.Error != nil {
		t.Fatalf("turn/start error: %#v", turnMsg.Error)
	}
	completed := readMessage(t, decoder, func(msg appserver.Message) bool { return msg.Method == "turn/completed" })
	if completed.Params == nil {
		t.Fatalf("turn/completed params missing: %#v", completed)
	}

	if err := inputWriter.Close(); err != nil {
		t.Fatalf("close input: %v", err)
	}
	if err := <-errCh; err != nil {
		t.Fatalf("app-server: %v\nstderr:\n%s", err, stderr.String())
	}
	_ = outputWriter.Close()
}

func mustEncode(t *testing.T, encoder *json.Encoder, value any) {
	t.Helper()
	if err := encoder.Encode(value); err != nil {
		t.Fatalf("encode request: %v", err)
	}
}

func readMessage(t *testing.T, decoder *json.Decoder, match func(appserver.Message) bool) appserver.Message {
	t.Helper()
	type result struct {
		msg appserver.Message
		err error
	}
	for {
		ch := make(chan result, 1)
		go func() {
			var msg appserver.Message
			err := decoder.Decode(&msg)
			ch <- result{msg: msg, err: err}
		}()
		select {
		case got := <-ch:
			if got.err != nil {
				t.Fatalf("decode message: %v", got.err)
			}
			if match(got.msg) {
				return got.msg
			}
		case <-time.After(2 * time.Second):
			t.Fatal("timed out waiting for appserver message")
		}
	}
}

func messageID(id any) string {
	switch value := id.(type) {
	case json.Number:
		return value.String()
	case float64:
		return fmt.Sprintf("%.0f", value)
	case string:
		return value
	default:
		return ""
	}
}
