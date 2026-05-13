package appserver

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"path/filepath"
	"slices"
	"strconv"
	"testing"
	"time"

	"github.com/pandelisz/gode/internal/godex"
)

func TestConnectionRequiresInitializeAndStreamsThreadTurnNotifications(t *testing.T) {
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
	server := New(app, Options{Version: "test"})
	conn := server.NewConnection(func(_ context.Context, msg Message) error {
		messages = append(messages, msg)
		return nil
	})

	if err := conn.HandleJSON(ctx, []byte(`{"id":1,"method":"thread/list"}`)); err != nil {
		t.Fatalf("handle before initialize: %v", err)
	}
	if got := responseErrorMessage(t, messages, 1); got != "Not initialized" {
		t.Fatalf("pre-init error = %q", got)
	}

	if err := conn.HandleJSON(ctx, []byte(`{"id":2,"method":"initialize","params":{"clientInfo":{"name":"gode_desktop_test","title":"Gode Desktop Test","version":"0.1.0"},"capabilities":{"experimentalApi":true}}}`)); err != nil {
		t.Fatalf("initialize: %v", err)
	}
	initResult := responseResult(t, messages, 2)
	if initResult["userAgent"] == "" {
		t.Fatalf("initialize userAgent missing: %#v", initResult)
	}
	if initResult["godeHome"] == "" {
		t.Fatalf("initialize godeHome missing: %#v", initResult)
	}
	if initResult["platformOs"] == "" {
		t.Fatalf("initialize platformOs missing: %#v", initResult)
	}

	if err := conn.HandleJSON(ctx, []byte(`{"method":"initialized"}`)); err != nil {
		t.Fatalf("initialized notification: %v", err)
	}
	if err := conn.HandleJSON(ctx, []byte(`{"id":3,"method":"thread/start","params":{"cwd":"`+app.Config.Workspace+`","model":"test-model"}}`)); err != nil {
		t.Fatalf("thread/start: %v", err)
	}
	threadResult := responseResult(t, messages, 3)
	thread := objectField(t, threadResult, "thread")
	threadID := stringField(t, thread, "id")
	if threadID == "" {
		t.Fatalf("thread id missing: %#v", thread)
	}
	if !hasNotification(messages, "thread/started") {
		t.Fatalf("missing thread/started notification: %#v", messages)
	}

	turnStart := map[string]any{
		"id":     4,
		"method": "turn/start",
		"params": map[string]any{
			"threadId": threadID,
			"input": []map[string]any{
				{"type": "text", "text": "hello from test"},
			},
		},
	}
	rawTurnStart, err := json.Marshal(turnStart)
	if err != nil {
		t.Fatalf("marshal turn/start: %v", err)
	}
	if err := conn.HandleJSON(ctx, rawTurnStart); err != nil {
		t.Fatalf("turn/start: %v", err)
	}
	waitFor(t, 2*time.Second, func() bool {
		return responseByID(messages, 4) != nil && hasNotification(messages, "turn/completed")
	})

	turnResponse := responseResult(t, messages, 4)
	turn := objectField(t, turnResponse, "turn")
	turnID := stringField(t, turn, "id")
	if turnID == "" {
		t.Fatalf("turn id missing: %#v", turn)
	}
	for _, method := range []string{
		"thread/status/changed",
		"turn/started",
		"item/agentMessage/delta",
		"turn/completed",
	} {
		if !hasNotification(messages, method) {
			t.Fatalf("missing %s notification in %#v", method, messages)
		}
	}

	if err := conn.HandleJSON(ctx, []byte(`{"id":5,"method":"thread/read","params":{"threadId":"`+threadID+`","includeTurns":true}}`)); err != nil {
		t.Fatalf("thread/read: %v", err)
	}
	readThread := objectField(t, responseResult(t, messages, 5), "thread")
	turns := sliceField(t, readThread, "turns")
	if len(turns) != 1 {
		t.Fatalf("turn count = %d, want 1: %#v", len(turns), readThread)
	}
}

func TestConnectionExposesFilesystemAndCommandExec(t *testing.T) {
	ctx := context.Background()
	workspace := filepath.Join(t.TempDir(), "workspace")
	app, err := godex.New(ctx, godex.Config{
		Workspace:   workspace,
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

	path := filepath.Join(workspace, "api.txt")
	writeReq := map[string]any{
		"id":     10,
		"method": "fs/writeFile",
		"params": map[string]any{
			"path":       path,
			"dataBase64": base64.StdEncoding.EncodeToString([]byte("hello api")),
		},
	}
	sendJSONRequest(t, conn, writeReq)
	if result := responseResult(t, messages, 10); len(result) != 0 {
		t.Fatalf("fs/writeFile result = %#v, want empty object", result)
	}

	sendJSONRequest(t, conn, map[string]any{
		"id":     11,
		"method": "fs/readFile",
		"params": map[string]any{"path": path},
	})
	readResult := responseResult(t, messages, 11)
	if readResult["dataBase64"] != base64.StdEncoding.EncodeToString([]byte("hello api")) {
		t.Fatalf("fs/readFile result = %#v", readResult)
	}

	sendJSONRequest(t, conn, map[string]any{
		"id":     12,
		"method": "fs/readDirectory",
		"params": map[string]any{"path": workspace},
	})
	entries := sliceField(t, responseResult(t, messages, 12), "entries")
	names := make([]string, 0, len(entries))
	for _, entry := range entries {
		names = append(names, stringField(t, entry.(map[string]any), "fileName"))
	}
	if !slices.Contains(names, "api.txt") {
		t.Fatalf("directory entries = %#v, want api.txt", entries)
	}

	sendJSONRequest(t, conn, map[string]any{
		"id":     13,
		"method": "command/exec",
		"params": map[string]any{
			"command": []string{"sh", "-c", "printf stdout; printf stderr >&2"},
			"cwd":     workspace,
		},
	})
	execResult := responseResult(t, messages, 13)
	if execResult["exitCode"] != float64(0) {
		t.Fatalf("exitCode = %#v", execResult["exitCode"])
	}
	if execResult["stdout"] != "stdout" {
		t.Fatalf("stdout = %#v", execResult["stdout"])
	}
	if execResult["stderr"] != "stderr" {
		t.Fatalf("stderr = %#v", execResult["stderr"])
	}
}

func TestModelListUsesBuiltInCatalog(t *testing.T) {
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
	resp := server.handleModelList().(map[string]any)
	models := resp["models"].([]map[string]any)
	if len(models) < 2 {
		t.Fatalf("models = %#v", models)
	}
	if models[0]["id"] != godex.DefaultModelID {
		t.Fatalf("first model id = %#v", models[0]["id"])
	}
	if models[0]["modelProvider"] != godex.ProviderOpenAI {
		t.Fatalf("first model provider = %#v", models[0]["modelProvider"])
	}
	if models[0]["defaultReasoningEffort"] != godex.ReasoningMedium {
		t.Fatalf("first model default reasoning = %#v", models[0]["defaultReasoningEffort"])
	}
}

func initializeTestConnection(t *testing.T, conn *Connection) {
	t.Helper()
	if err := conn.HandleJSON(context.Background(), []byte(`{"id":1,"method":"initialize","params":{"clientInfo":{"name":"gode_test","title":"Gode Test","version":"0.1.0"}}}`)); err != nil {
		t.Fatalf("initialize: %v", err)
	}
	if err := conn.HandleJSON(context.Background(), []byte(`{"method":"initialized"}`)); err != nil {
		t.Fatalf("initialized: %v", err)
	}
}

func sendJSONRequest(t *testing.T, conn *Connection, request map[string]any) {
	t.Helper()
	raw, err := json.Marshal(request)
	if err != nil {
		t.Fatalf("marshal request: %v", err)
	}
	if err := conn.HandleJSON(context.Background(), raw); err != nil {
		t.Fatalf("handle %s: %v", request["method"], err)
	}
}

func responseResult(t *testing.T, messages []Message, id any) map[string]any {
	t.Helper()
	msg := responseByID(messages, id)
	if msg == nil {
		t.Fatalf("missing response id %v in %#v", id, messages)
	}
	if msg.Error != nil {
		t.Fatalf("response id %v error = %#v", id, msg.Error)
	}
	result, ok := msg.Result.(map[string]any)
	if !ok {
		t.Fatalf("response result id %v = %#v", id, msg.Result)
	}
	return result
}

func responseErrorMessage(t *testing.T, messages []Message, id any) string {
	t.Helper()
	msg := responseByID(messages, id)
	if msg == nil {
		t.Fatalf("missing response id %v in %#v", id, messages)
	}
	if msg.Error == nil {
		t.Fatalf("response id %v error missing: %#v", id, msg)
	}
	return msg.Error.Message
}

func responseByID(messages []Message, id any) *Message {
	want := idKey(id)
	for i := range messages {
		if idKey(messages[i].ID) == want && (messages[i].Result != nil || messages[i].Error != nil) {
			return &messages[i]
		}
	}
	return nil
}

func hasNotification(messages []Message, method string) bool {
	for _, msg := range messages {
		if msg.Method == method && msg.ID == nil {
			return true
		}
	}
	return false
}

func objectField(t *testing.T, object map[string]any, field string) map[string]any {
	t.Helper()
	value, ok := object[field].(map[string]any)
	if !ok {
		t.Fatalf("%s = %#v", field, object[field])
	}
	return value
}

func sliceField(t *testing.T, object map[string]any, field string) []any {
	t.Helper()
	value, ok := object[field].([]any)
	if !ok {
		t.Fatalf("%s = %#v", field, object[field])
	}
	return value
}

func stringField(t *testing.T, object map[string]any, field string) string {
	t.Helper()
	value, ok := object[field].(string)
	if !ok {
		t.Fatalf("%s = %#v", field, object[field])
	}
	return value
}

func waitFor(t *testing.T, timeout time.Duration, condition func() bool) {
	t.Helper()
	deadline := time.Now().Add(timeout)
	for time.Now().Before(deadline) {
		if condition() {
			return
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatal("condition did not become true before timeout")
}

func idKey(id any) string {
	switch value := id.(type) {
	case json.Number:
		return value.String()
	case float64:
		return strconv.FormatInt(int64(value), 10)
	case int:
		return strconv.Itoa(value)
	case string:
		return value
	default:
		return ""
	}
}
