package appserver

import (
	"context"
	"os"
	"path/filepath"
	"testing"

	"github.com/pandelisz/gode/internal/godex"
)

func TestSkillsHandlersListReadAndToggle(t *testing.T) {
	ctx := context.Background()
	workspace := filepath.Join(t.TempDir(), "workspace")
	skillPath := filepath.Join(workspace, ".agents", "skills", "go-tests", "SKILL.md")
	writeAppServerSkill(t, skillPath, "go-tests", "Run tests")
	app, err := godex.New(ctx, godex.Config{
		Workspace:   workspace,
		DataDir:     t.TempDir(),
		HomeDir:     t.TempDir(),
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

	sendJSONRequest(t, conn, map[string]any{"id": 2, "method": "skills/list"})
	skills := sliceField(t, responseResult(t, messages, 2), "skills")
	if len(skills) != 1 || stringField(t, skills[0].(map[string]any), "name") != "go-tests" {
		t.Fatalf("skills = %#v", skills)
	}

	sendJSONRequest(t, conn, map[string]any{"id": 3, "method": "skill/read", "params": map[string]any{"path": skillPath}})
	if content := responseResult(t, messages, 3)["content"]; content == "" {
		t.Fatalf("read result = %#v", responseResult(t, messages, 3))
	}

	sendJSONRequest(t, conn, map[string]any{"id": 4, "method": "skill/setEnabled", "params": map[string]any{"path": skillPath, "enabled": false}})
	if result := responseResult(t, messages, 4); len(result) != 0 {
		t.Fatalf("set result = %#v", result)
	}
	if notificationByMethod(messages, "skills/changed") == nil {
		t.Fatalf("missing skills/changed notification: %#v", messages)
	}
	messages = nil
	sendJSONRequest(t, conn, map[string]any{"id": 5, "method": "skills/list"})
	skills = sliceField(t, responseResult(t, messages, 5), "skills")
	if enabled := skills[0].(map[string]any)["enabled"]; enabled != false {
		t.Fatalf("enabled = %#v skills=%#v", enabled, skills)
	}
}

func writeAppServerSkill(t *testing.T, path string, name string, description string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatalf("mkdir skill: %v", err)
	}
	if err := os.WriteFile(path, []byte("---\nname: "+name+"\ndescription: "+description+"\n---\n"), 0o644); err != nil {
		t.Fatalf("write skill: %v", err)
	}
}
