package appserver

import (
	"context"
	"path/filepath"
	"testing"
	"time"

	"github.com/pandelisz/gode/internal/godex"
)

func TestThreadGoalProtocol(t *testing.T) {
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

	var messages []Message
	conn := New(app, Options{Version: "test"}).NewConnection(func(_ context.Context, msg Message) error {
		messages = append(messages, msg)
		return nil
	})
	initializeTestConnection(t, conn)
	sendJSONRequest(t, conn, map[string]any{"id": 20, "method": "thread/start", "params": map[string]any{"cwd": app.Config.Workspace}})
	threadID := stringField(t, objectField(t, responseResult(t, messages, 20), "thread"), "id")

	sendJSONRequest(t, conn, map[string]any{"id": 21, "method": "thread/goal/get", "params": map[string]any{"threadId": threadID}})
	if goal := responseResult(t, messages, 21)["goal"]; goal != nil {
		t.Fatalf("empty goal = %#v", goal)
	}
	sendJSONRequest(t, conn, map[string]any{"id": 22, "method": "thread/goal/set", "params": map[string]any{"threadId": threadID, "objective": "ship it", "tokenBudget": 10000}})
	goal := objectField(t, responseResult(t, messages, 22), "goal")
	if stringField(t, goal, "objective") != "ship it" || stringField(t, goal, "status") != "active" {
		t.Fatalf("set goal = %#v", goal)
	}
	if goal["tokenBudget"] != float64(10000) {
		t.Fatalf("tokenBudget = %#v", goal["tokenBudget"])
	}
	waitFor(t, time.Second, func() bool {
		return hasNotification(messages, "thread/goal/updated")
	})

	sendJSONRequest(t, conn, map[string]any{"id": 23, "method": "thread/goal/set", "params": map[string]any{"threadId": threadID, "status": "paused"}})
	goal = objectField(t, responseResult(t, messages, 23), "goal")
	if stringField(t, goal, "status") != "paused" {
		t.Fatalf("paused goal = %#v", goal)
	}
	sendJSONRequest(t, conn, map[string]any{"id": 24, "method": "thread/goal/set", "params": map[string]any{"threadId": threadID, "status": "active"}})
	goal = objectField(t, responseResult(t, messages, 24), "goal")
	if stringField(t, goal, "status") != "active" {
		t.Fatalf("resumed goal = %#v", goal)
	}
	sendJSONRequest(t, conn, map[string]any{"id": 25, "method": "thread/goal/clear", "params": map[string]any{"threadId": threadID}})
	if result := responseResult(t, messages, 25); len(result) != 0 {
		t.Fatalf("clear result = %#v", result)
	}
	waitFor(t, time.Second, func() bool {
		return hasNotification(messages, "thread/goal/cleared")
	})
}

func TestThreadGoalSetValidation(t *testing.T) {
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

	var messages []Message
	conn := New(app, Options{Version: "test"}).NewConnection(func(_ context.Context, msg Message) error {
		messages = append(messages, msg)
		return nil
	})
	initializeTestConnection(t, conn)
	sendJSONRequest(t, conn, map[string]any{"id": 10, "method": "thread/goal/set", "params": map[string]any{"threadId": "s1", "objective": ""}})
	if got := responseErrorMessage(t, messages, 10); got == "" {
		t.Fatalf("expected empty objective error")
	}
	sendJSONRequest(t, conn, map[string]any{"id": 11, "method": "thread/goal/set", "params": map[string]any{"threadId": "s1", "objective": "ship", "tokenBudget": -1}})
	if got := responseErrorMessage(t, messages, 11); got == "" {
		t.Fatalf("expected invalid budget error")
	}
}
