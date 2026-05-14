package appserver

import (
	"context"
	"encoding/base64"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex"
)

func TestFSUploadWritesFileAtomically(t *testing.T) {
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

	path := filepath.Join(workspace, ".gode", "uploads", "thread-1", "notes.txt")
	sendJSONRequest(t, conn, map[string]any{
		"id":     10,
		"method": "fs/upload/start",
		"params": map[string]any{"path": path, "sizeBytes": 11},
	})
	uploadID := stringField(t, responseResult(t, messages, 10), "uploadId")
	if uploadID == "" {
		t.Fatalf("upload id missing")
	}
	sendJSONRequest(t, conn, map[string]any{
		"id":     11,
		"method": "fs/upload/chunk",
		"params": map[string]any{
			"uploadId":   uploadID,
			"offset":     0,
			"dataBase64": base64.StdEncoding.EncodeToString([]byte("hello ")),
		},
	})
	sendJSONRequest(t, conn, map[string]any{
		"id":     12,
		"method": "fs/upload/chunk",
		"params": map[string]any{
			"uploadId":   uploadID,
			"offset":     6,
			"dataBase64": base64.StdEncoding.EncodeToString([]byte("world")),
		},
	})
	sendJSONRequest(t, conn, map[string]any{
		"id":     13,
		"method": "fs/upload/finish",
		"params": map[string]any{"uploadId": uploadID},
	})
	result := responseResult(t, messages, 13)
	if result["path"] != path || result["sizeBytes"] != float64(11) {
		t.Fatalf("finish result = %#v", result)
	}
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read uploaded file: %v", err)
	}
	if string(data) != "hello world" {
		t.Fatalf("uploaded data = %q", data)
	}
}

func TestFSUploadRejectsOffsetMismatchAndCancelRemovesTempFile(t *testing.T) {
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
	server := New(app, Options{Version: "test"})
	conn := server.NewConnection(func(_ context.Context, msg Message) error {
		messages = append(messages, msg)
		return nil
	})
	initializeTestConnection(t, conn)

	path := filepath.Join(workspace, ".gode", "uploads", "thread-1", "bad.txt")
	sendJSONRequest(t, conn, map[string]any{
		"id":     20,
		"method": "fs/upload/start",
		"params": map[string]any{"path": path},
	})
	uploadID := stringField(t, responseResult(t, messages, 20), "uploadId")
	sendJSONRequest(t, conn, map[string]any{
		"id":     21,
		"method": "fs/upload/chunk",
		"params": map[string]any{
			"uploadId":   uploadID,
			"offset":     1,
			"dataBase64": base64.StdEncoding.EncodeToString([]byte("x")),
		},
	})
	if got := responseErrorMessage(t, messages, 21); !strings.Contains(got, "offset mismatch") {
		t.Fatalf("offset error = %q", got)
	}
	upload := server.uploadByID(uploadID)
	if upload == nil {
		t.Fatal("upload should still exist before cancel")
	}
	tmp := upload.TempPath
	sendJSONRequest(t, conn, map[string]any{
		"id":     22,
		"method": "fs/upload/cancel",
		"params": map[string]any{"uploadId": uploadID},
	})
	if _, err := os.Stat(tmp); !os.IsNotExist(err) {
		t.Fatalf("temp file should be removed, stat err=%v", err)
	}
}
