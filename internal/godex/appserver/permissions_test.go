package appserver

import (
	"context"
	"path/filepath"
	"testing"
	"time"

	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/tools"
)

func TestPermissionRespondApprovesPendingToolCall(t *testing.T) {
	ctx := context.Background()
	workspace := filepath.Join(t.TempDir(), "workspace")
	app, err := godex.New(ctx, godex.Config{
		Workspace: workspace,
		DataDir:   t.TempDir(),
		Provider:  "mock",
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

	resultCh := make(chan error, 1)
	go func() {
		_, err := app.Tools.Run(ctx, tools.Call{
			Name:      "write_file",
			SessionID: "thread-permission",
			RunID:     "turn-permission",
			Input: map[string]any{
				"path":    "approved.txt",
				"content": "approved by appserver",
			},
		})
		resultCh <- err
	}()

	waitFor(t, time.Second, func() bool {
		return notificationByMethod(messages, "permission/requested") != nil
	})
	notification := notificationByMethod(messages, "permission/requested")
	correlationID := stringField(t, notification.Params.(map[string]any), "correlationId")
	if correlationID == "" {
		t.Fatalf("permission notification missing correlation id: %#v", notification)
	}

	sendJSONRequest(t, conn, map[string]any{
		"id":     6,
		"method": "permission/respond",
		"params": map[string]any{
			"correlationId":   correlationID,
			"approved":        true,
			"allowForSession": true,
			"reason":          "test approval",
		},
	})
	if result := responseResult(t, messages, 6); len(result) != 0 {
		t.Fatalf("permission/respond result = %#v", result)
	}

	select {
	case err := <-resultCh:
		if err != nil {
			t.Fatalf("tool run: %v", err)
		}
	case <-time.After(time.Second):
		t.Fatal("timed out waiting for tool run")
	}
}

func notificationByMethod(messages []Message, method string) *Message {
	for i := range messages {
		if messages[i].Method == method && messages[i].ID == nil {
			return &messages[i]
		}
	}
	return nil
}
