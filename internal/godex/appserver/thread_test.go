package appserver

import (
	"context"
	"encoding/json"
	"path/filepath"
	"testing"
	"time"

	"github.com/pandelisz/gode/internal/godex"
)

func TestThreadPersistenceAcrossServerInstances(t *testing.T) {
	ctx := context.Background()
	dataDir := t.TempDir()
	workspace := filepath.Join(t.TempDir(), "workspace")
	app, err := godex.New(ctx, godex.Config{
		Workspace:   workspace,
		DataDir:     dataDir,
		Provider:    "mock",
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("new app: %v", err)
	}

	var firstMessages []Message
	first := New(app, Options{Version: "test"}).NewConnection(func(_ context.Context, msg Message) error {
		firstMessages = append(firstMessages, msg)
		return nil
	})
	initializeTestConnection(t, first)
	sendJSONRequest(t, first, map[string]any{"id": 20, "method": "thread/start", "params": map[string]any{"cwd": workspace}})
	threadID := stringField(t, objectField(t, responseResult(t, firstMessages, 20), "thread"), "id")
	startTurn(t, first, &firstMessages, 21, threadID, "persist me")
	if err := app.Close(ctx); err != nil {
		t.Fatalf("close first app: %v", err)
	}

	reopened, err := godex.New(ctx, godex.Config{
		Workspace:   workspace,
		DataDir:     dataDir,
		Provider:    "mock",
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("reopen app: %v", err)
	}
	defer reopened.Close(ctx)

	var secondMessages []Message
	second := New(reopened, Options{Version: "test"}).NewConnection(func(_ context.Context, msg Message) error {
		secondMessages = append(secondMessages, msg)
		return nil
	})
	initializeTestConnection(t, second)
	sendJSONRequest(t, second, map[string]any{"id": 22, "method": "thread/list", "params": map[string]any{}})
	listed := sliceField(t, responseResult(t, secondMessages, 22), "data")
	if len(listed) != 1 || stringField(t, listed[0].(map[string]any), "id") != threadID {
		t.Fatalf("thread/list after reopen = %#v", listed)
	}
	sendJSONRequest(t, second, map[string]any{"id": 23, "method": "thread/read", "params": map[string]any{"threadId": threadID, "includeTurns": true}})
	thread := objectField(t, responseResult(t, secondMessages, 23), "thread")
	turns := sliceField(t, thread, "turns")
	if len(turns) != 1 {
		t.Fatalf("turns after reopen = %#v", turns)
	}
}

func TestTurnResumePersistsMultipleTurnsOnThread(t *testing.T) {
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
	sendJSONRequest(t, conn, map[string]any{"id": 30, "method": "thread/start", "params": map[string]any{"cwd": app.Config.Workspace}})
	threadID := stringField(t, objectField(t, responseResult(t, messages, 30), "thread"), "id")
	startTurn(t, conn, &messages, 31, threadID, "first")
	startTurn(t, conn, &messages, 32, threadID, "second")

	sendJSONRequest(t, conn, map[string]any{"id": 33, "method": "thread/read", "params": map[string]any{"threadId": threadID, "includeTurns": true}})
	thread := objectField(t, responseResult(t, messages, 33), "thread")
	turns := sliceField(t, thread, "turns")
	if len(turns) != 2 {
		t.Fatalf("turns = %#v", turns)
	}
	stored, err := app.Messages.ListBySession(ctx, threadID)
	if err != nil {
		t.Fatalf("list messages: %v", err)
	}
	if len(stored) != 4 {
		t.Fatalf("stored messages = %#v", stored)
	}
}

func TestTurnInterruptCancelsOnlyActiveThread(t *testing.T) {
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
	firstCtx, cancelFirst := context.WithCancel(context.Background())
	defer cancelFirst()
	secondCtx, cancelSecond := context.WithCancel(context.Background())
	defer cancelSecond()
	server.threads["first"] = &threadState{Thread: Thread{ID: "first"}, activeCancel: cancelFirst}
	server.threads["second"] = &threadState{Thread: Thread{ID: "second"}, activeCancel: cancelSecond}

	_, rpcErr := server.handleTurnInterrupt(ctx, mustRaw(t, map[string]any{"threadId": "first", "turnId": "turn-1"}))
	if rpcErr != nil {
		t.Fatalf("interrupt: %#v", rpcErr)
	}
	select {
	case <-firstCtx.Done():
	case <-time.After(time.Second):
		t.Fatal("first thread was not cancelled")
	}
	select {
	case <-secondCtx.Done():
		t.Fatal("second thread should not be cancelled")
	default:
	}
}

func TestTurnSteerRequiresActiveTurn(t *testing.T) {
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
	_, rpcErr := server.handleTurnSteer(ctx, mustRaw(t, map[string]any{
		"threadId":       "missing",
		"expectedTurnId": "turn-1",
		"input":          []map[string]any{{"type": "text", "text": "steer"}},
	}))
	if rpcErr == nil || rpcErr.Message != "no active turn to steer" {
		t.Fatalf("rpc err = %#v", rpcErr)
	}
}

func startTurn(t *testing.T, conn *Connection, messages *[]Message, id int, threadID string, text string) {
	t.Helper()
	before := len(*messages)
	request := map[string]any{
		"id":     id,
		"method": "turn/start",
		"params": map[string]any{
			"threadId": threadID,
			"input": []map[string]any{
				{"type": "text", "text": text},
			},
		},
	}
	sendJSONRequest(t, conn, request)
	waitFor(t, 2*time.Second, func() bool {
		return responseByID(*messages, id) != nil && notificationCount((*messages)[before:], "turn/completed") > 0
	})
}

func mustRaw(t *testing.T, value any) json.RawMessage {
	t.Helper()
	data, err := json.Marshal(value)
	if err != nil {
		t.Fatalf("marshal raw: %v", err)
	}
	return data
}

func notificationCount(messages []Message, method string) int {
	count := 0
	for _, msg := range messages {
		if msg.Method == method && msg.ID == nil {
			count++
		}
	}
	return count
}
