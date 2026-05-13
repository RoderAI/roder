package appserver

import (
	"context"
	"encoding/json"
	"net/http"
	"path/filepath"
	"testing"

	"github.com/pandelisz/gode/internal/godex"
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
