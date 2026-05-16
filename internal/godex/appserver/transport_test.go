package appserver

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/eventbus"
	"nhooyr.io/websocket"
)

func TestParseListenURL(t *testing.T) {
	tests := []struct {
		raw  string
		kind TransportKind
	}{
		{raw: "stdio://", kind: TransportStdio},
		{raw: "ws://127.0.0.1:0", kind: TransportWebSocket},
		{raw: "off", kind: TransportOff},
	}
	for _, tt := range tests {
		got, err := ParseListenURL(tt.raw)
		if err != nil {
			t.Fatalf("ParseListenURL(%q): %v", tt.raw, err)
		}
		if got.Kind != tt.kind {
			t.Fatalf("ParseListenURL(%q).Kind = %v, want %v", tt.raw, got.Kind, tt.kind)
		}
	}
	if _, err := ParseListenURL("http://127.0.0.1:1"); err == nil {
		t.Fatal("ParseListenURL accepted unsupported scheme")
	}
}

func TestRemoteWebSocketRequiresAuth(t *testing.T) {
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

	auth, err := NewRemoteAuth("remote-secret", time.Unix(10, 0))
	if err != nil {
		t.Fatalf("new auth: %v", err)
	}
	server := New(app, Options{Version: "test", Remote: RemoteOptions{Enabled: true, Auth: auth}})
	listener, err := server.ListenWebSocket(ctx, "127.0.0.1:0")
	if err != nil {
		t.Fatalf("listen websocket: %v", err)
	}
	defer listener.Close(ctx)

	resp, err := http.Get(listener.HTTPURL() + "/readyz")
	if err != nil {
		t.Fatalf("readyz: %v", err)
	}
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("readyz status = %d", resp.StatusCode)
	}
	_ = resp.Body.Close()

	authFailedCh := app.Bus.Subscribe(ctx, eventbus.Filter{Kinds: []eventbus.Kind{KindRemoteAuthFailed}})
	_, resp, err = websocket.Dial(ctx, listener.WebSocketURL(), nil)
	if err == nil {
		t.Fatal("unauthenticated dial succeeded")
	}
	if resp == nil || resp.StatusCode != http.StatusUnauthorized {
		t.Fatalf("unauthenticated status = %v err=%v", statusCode(resp), err)
	}
	authFailed := awaitRemoteEvent(t, ctx, authFailedCh)
	payload, err := json.Marshal(authFailed.Payload)
	if err != nil {
		t.Fatalf("marshal auth failed payload: %v", err)
	}
	if string(payload) == "" || string(payload) == "null" {
		t.Fatalf("auth failed payload missing: %#v", authFailed)
	}
	if bytes.Contains(payload, []byte("remote-secret")) {
		t.Fatalf("auth failed payload leaked token: %s", payload)
	}

	connectedCh := app.Bus.Subscribe(ctx, eventbus.Filter{Kinds: []eventbus.Kind{KindRemoteClientConnected}})
	ws, _, err := websocket.Dial(ctx, listener.WebSocketURL(), &websocket.DialOptions{
		HTTPHeader: http.Header{"Authorization": []string{"Bearer remote-secret"}},
	})
	if err != nil {
		t.Fatalf("authorized dial: %v", err)
	}
	defer ws.Close(websocket.StatusNormalClosure, "")
	_ = awaitRemoteEvent(t, ctx, connectedCh)
	assertInitializeHasRemote(t, ctx, ws)
}

func TestRemoteWebSocketSubprotocolAuth(t *testing.T) {
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

	auth, err := NewRemoteAuth("remote-secret", time.Unix(10, 0))
	if err != nil {
		t.Fatalf("new auth: %v", err)
	}
	server := New(app, Options{Version: "test", Remote: RemoteOptions{Enabled: true, Auth: auth}})
	listener, err := server.ListenWebSocket(ctx, "127.0.0.1:0")
	if err != nil {
		t.Fatalf("listen websocket: %v", err)
	}
	defer listener.Close(ctx)

	ws, _, err := websocket.Dial(ctx, listener.WebSocketURL(), &websocket.DialOptions{
		Subprotocols: []string{remoteSubprotocol, "bearer.remote-secret"},
	})
	if err != nil {
		t.Fatalf("subprotocol dial: %v", err)
	}
	defer ws.Close(websocket.StatusNormalClosure, "")
	if ws.Subprotocol() != remoteSubprotocol {
		t.Fatalf("subprotocol = %q", ws.Subprotocol())
	}
	assertInitializeHasRemote(t, ctx, ws)
}

func TestRemoteWebSocketAllowsExpoOriginByDefault(t *testing.T) {
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

	auth, err := NewRemoteAuth("remote-secret", time.Unix(10, 0))
	if err != nil {
		t.Fatalf("new auth: %v", err)
	}
	server := New(app, Options{Version: "test", Remote: RemoteOptions{Enabled: true, Auth: auth}})
	listener, err := server.ListenWebSocket(ctx, "127.0.0.1:0")
	if err != nil {
		t.Fatalf("listen websocket: %v", err)
	}
	defer listener.Close(ctx)

	ws, _, err := websocket.Dial(ctx, listener.WebSocketURL(), &websocket.DialOptions{
		HTTPHeader:   http.Header{"Origin": []string{"http://10.13.37.30:8081"}},
		Subprotocols: []string{remoteSubprotocol, "bearer.remote-secret"},
	})
	if err != nil {
		t.Fatalf("expo-origin dial: %v", err)
	}
	defer ws.Close(websocket.StatusNormalClosure, "")
	assertInitializeHasRemote(t, ctx, ws)
}

func TestRemoteWebSocketLogsConnectionLifecycle(t *testing.T) {
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

	auth, err := NewRemoteAuth("remote-secret", time.Unix(10, 0))
	if err != nil {
		t.Fatalf("new auth: %v", err)
	}
	var logs bytes.Buffer
	server := New(app, Options{Version: "test", Remote: RemoteOptions{Enabled: true, Auth: auth}, Log: &logs})
	listener, err := server.ListenWebSocket(ctx, "127.0.0.1:0")
	if err != nil {
		t.Fatalf("listen websocket: %v", err)
	}
	defer listener.Close(ctx)

	_, _, _ = websocket.Dial(ctx, listener.WebSocketURL(), nil)
	ws, _, err := websocket.Dial(ctx, listener.WebSocketURL(), &websocket.DialOptions{
		Subprotocols: []string{remoteSubprotocol, "bearer.remote-secret"},
	})
	if err != nil {
		t.Fatalf("subprotocol dial: %v", err)
	}
	_ = ws.Close(websocket.StatusNormalClosure, "test done")

	got := logs.String()
	for _, want := range []string{"remote ", "dial ", "auth_failed", "connected", "disconnected", "auth=subprotocol"} {
		if !strings.Contains(got, want) {
			t.Fatalf("logs missing %q:\n%s", want, got)
		}
	}
	if strings.Contains(got, "remote-secret") {
		t.Fatalf("logs leaked token:\n%s", got)
	}
}

func TestRemoteWebSocketMockTurn(t *testing.T) {
	ctx := context.Background()
	workspace := filepath.Join(t.TempDir(), "workspace")
	app, err := godex.New(ctx, godex.Config{
		Workspace:   workspace,
		DataDir:     t.TempDir(),
		Provider:    "mock",
		Model:       "mock",
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("new app: %v", err)
	}
	defer app.Close(ctx)

	auth, err := NewRemoteAuth("remote-secret", time.Unix(10, 0))
	if err != nil {
		t.Fatalf("new auth: %v", err)
	}
	server := New(app, Options{Version: "test", Remote: RemoteOptions{Enabled: true, Auth: auth}})
	listener, err := server.ListenWebSocket(ctx, "127.0.0.1:0")
	if err != nil {
		t.Fatalf("listen websocket: %v", err)
	}
	defer listener.Close(ctx)

	ws, _, err := websocket.Dial(ctx, listener.WebSocketURL(), &websocket.DialOptions{
		HTTPHeader: http.Header{"Authorization": []string{"Bearer remote-secret"}},
	})
	if err != nil {
		t.Fatalf("authorized dial: %v", err)
	}
	defer ws.Close(websocket.StatusNormalClosure, "")

	writeWS(t, ctx, ws, map[string]any{
		"id":     1,
		"method": "initialize",
		"params": map[string]any{"clientInfo": map[string]any{"name": "remote-smoke"}},
	})
	initMsg := readWSMessage(t, ctx, ws, func(msg Message) bool { return messageID(msg.ID) == "1" })
	if initMsg.Error != nil {
		t.Fatalf("initialize error: %#v", initMsg.Error)
	}
	writeWS(t, ctx, ws, map[string]any{"method": "initialized"})
	writeWS(t, ctx, ws, map[string]any{
		"id":     2,
		"method": "thread/start",
		"params": map[string]any{"cwd": workspace, "model": "mock"},
	})
	threadMsg := readWSMessage(t, ctx, ws, func(msg Message) bool { return messageID(msg.ID) == "2" })
	if threadMsg.Error != nil {
		t.Fatalf("thread/start error: %#v", threadMsg.Error)
	}
	threadID := threadMsg.Result.(map[string]any)["thread"].(map[string]any)["id"].(string)
	writeWS(t, ctx, ws, map[string]any{
		"id":     3,
		"method": "turn/start",
		"params": map[string]any{
			"threadId": threadID,
			"input": []map[string]any{
				{"type": "text", "text": "hello remote"},
			},
		},
	})
	turnMsg := readWSMessage(t, ctx, ws, func(msg Message) bool { return messageID(msg.ID) == "3" })
	if turnMsg.Error != nil {
		t.Fatalf("turn/start error: %#v", turnMsg.Error)
	}
	completed := readWSMessage(t, ctx, ws, func(msg Message) bool { return msg.Method == "turn/completed" })
	if completed.Params == nil {
		t.Fatalf("turn/completed params missing: %#v", completed)
	}
}

func assertInitializeHasRemote(t *testing.T, ctx context.Context, ws *websocket.Conn) {
	t.Helper()
	if err := ws.Write(ctx, websocket.MessageText, []byte(`{"id":1,"method":"initialize","params":{"clientInfo":{"name":"gode_ws_test"}}}`)); err != nil {
		t.Fatalf("write initialize: %v", err)
	}
	_, data, err := ws.Read(ctx)
	if err != nil {
		t.Fatalf("read initialize response: %v", err)
	}
	var msg Message
	if err := json.Unmarshal(data, &msg); err != nil {
		t.Fatalf("unmarshal response: %v", err)
	}
	if msg.Error != nil {
		t.Fatalf("initialize error: %#v", msg.Error)
	}
	result := msg.Result.(map[string]any)
	if result["remote"] == nil {
		t.Fatalf("remote capability missing: %#v", result)
	}
}

func statusCode(resp *http.Response) int {
	if resp == nil {
		return 0
	}
	return resp.StatusCode
}

func awaitRemoteEvent(t *testing.T, ctx context.Context, ch <-chan eventbus.Event) eventbus.Event {
	t.Helper()
	deadline, cancel := context.WithTimeout(ctx, 2*time.Second)
	defer cancel()
	select {
	case ev := <-ch:
		return ev
	case <-deadline.Done():
		t.Fatalf("timed out waiting for remote event: %v", deadline.Err())
		return eventbus.Event{}
	}
}

func writeWS(t *testing.T, ctx context.Context, ws *websocket.Conn, value any) {
	t.Helper()
	data, err := json.Marshal(value)
	if err != nil {
		t.Fatalf("marshal websocket message: %v", err)
	}
	if err := ws.Write(ctx, websocket.MessageText, data); err != nil {
		t.Fatalf("write websocket message: %v", err)
	}
}

func readWSMessage(t *testing.T, ctx context.Context, ws *websocket.Conn, match func(Message) bool) Message {
	t.Helper()
	deadline, cancel := context.WithTimeout(ctx, 2*time.Second)
	defer cancel()
	for {
		_, data, err := ws.Read(deadline)
		if err != nil {
			t.Fatalf("read websocket message: %v", err)
		}
		var msg Message
		if err := json.Unmarshal(data, &msg); err != nil {
			t.Fatalf("unmarshal websocket message: %v", err)
		}
		if match(msg) {
			return msg
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

func TestWebSocketTransportHealthOriginAndInitialize(t *testing.T) {
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

	server := New(app, Options{Version: "test"})
	listener, err := server.ListenWebSocket(ctx, "127.0.0.1:0")
	if err != nil {
		t.Fatalf("listen websocket: %v", err)
	}
	defer listener.Close(ctx)

	resp, err := http.Get(listener.HTTPURL() + "/readyz")
	if err != nil {
		t.Fatalf("readyz: %v", err)
	}
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("readyz status = %d", resp.StatusCode)
	}
	_ = resp.Body.Close()

	req, err := http.NewRequestWithContext(ctx, http.MethodGet, listener.HTTPURL()+"/healthz", nil)
	if err != nil {
		t.Fatalf("new request: %v", err)
	}
	req.Header.Set("Origin", "https://example.test")
	resp, err = http.DefaultClient.Do(req)
	if err != nil {
		t.Fatalf("healthz with origin: %v", err)
	}
	if resp.StatusCode != http.StatusForbidden {
		t.Fatalf("healthz with origin status = %d", resp.StatusCode)
	}
	_ = resp.Body.Close()

	ws, _, err := websocket.Dial(ctx, listener.WebSocketURL(), nil)
	if err != nil {
		t.Fatalf("dial websocket: %v", err)
	}
	defer ws.Close(websocket.StatusNormalClosure, "")

	if err := ws.Write(ctx, websocket.MessageText, []byte(`{"id":1,"method":"initialize","params":{"clientInfo":{"name":"gode_ws_test","title":"Gode WS Test","version":"0.1.0"}}}`)); err != nil {
		t.Fatalf("write initialize: %v", err)
	}
	_, data, err := ws.Read(ctx)
	if err != nil {
		t.Fatalf("read initialize response: %v", err)
	}
	var msg Message
	if err := json.Unmarshal(data, &msg); err != nil {
		t.Fatalf("unmarshal response: %v", err)
	}
	if msg.Error != nil {
		t.Fatalf("initialize error: %#v", msg.Error)
	}
	if msg.Result == nil {
		t.Fatalf("initialize result missing: %#v", msg)
	}
}
