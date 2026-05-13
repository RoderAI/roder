package godex

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex/eventbus"
)

func TestNewAppWiresBroadCoreWithMockProvider(t *testing.T) {
	app, err := New(context.Background(), Config{
		Workspace:   filepath.Join(t.TempDir(), "workspace"),
		DataDir:     t.TempDir(),
		Provider:    "mock",
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("new app: %v", err)
	}
	defer app.Close(context.Background())

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	events := app.Bus.Subscribe(ctx, eventbus.Filter{Kinds: []eventbus.Kind{eventbus.KindRunCompleted}})

	result, err := app.RunPrompt(context.Background(), "hello")
	if err != nil {
		t.Fatalf("run: %v", err)
	}

	select {
	case ev := <-events:
		if ev.Kind != eventbus.KindRunCompleted {
			t.Fatalf("kind = %q", ev.Kind)
		}
	default:
		t.Fatal("expected run completed event")
	}

	messages, err := app.Messages.ListBySession(context.Background(), result.SessionID)
	if err != nil {
		t.Fatalf("messages: %v", err)
	}
	if len(messages) != 2 || messages[0].Text != "hello" || messages[1].Text != "mock response" {
		t.Fatalf("messages = %#v", messages)
	}
	session, ok, err := app.Sessions.Get(context.Background(), result.SessionID)
	if err != nil {
		t.Fatalf("session: %v", err)
	}
	if !ok || session.MessageCount != 2 {
		t.Fatalf("session = %#v ok=%v", session, ok)
	}
}

func TestNewAppLoadsRepoContextMessages(t *testing.T) {
	workspace := filepath.Join(t.TempDir(), "workspace")
	if err := os.MkdirAll(workspace, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(workspace, "AGENTS.md"), []byte("repo rules"), 0o644); err != nil {
		t.Fatalf("write agents: %v", err)
	}
	if err := os.WriteFile(filepath.Join(workspace, ".gode.toml"), []byte(`[agent]
extra_context = "inline repo context"
`), 0o644); err != nil {
		t.Fatalf("write config: %v", err)
	}

	app, err := New(context.Background(), Config{
		Workspace:   workspace,
		DataDir:     t.TempDir(),
		Provider:    "mock",
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("new app: %v", err)
	}
	defer app.Close(context.Background())

	joined := ""
	for _, msg := range app.contextMessages {
		joined += msg.Content + "\n"
	}
	for _, want := range []string{"repo rules", "AGENTS.md", "inline repo context"} {
		if !strings.Contains(joined, want) {
			t.Fatalf("context messages missing %q:\n%s", want, joined)
		}
	}
}

func TestNewAppDiscoversProjectSkills(t *testing.T) {
	workspace := filepath.Join(t.TempDir(), "workspace")
	skillDir := filepath.Join(workspace, ".agents", "skills", "go-tests")
	if err := os.MkdirAll(skillDir, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(skillDir, "SKILL.md"), []byte(`---
name: go-tests
description: Run tests
---
Run Go tests.
`), 0o644); err != nil {
		t.Fatalf("write skill: %v", err)
	}
	app, err := New(context.Background(), Config{
		Workspace:   workspace,
		DataDir:     t.TempDir(),
		Provider:    "mock",
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("new app: %v", err)
	}
	defer app.Close(context.Background())

	if len(app.skills) != 1 || app.skills[0].Name != "go-tests" {
		t.Fatalf("skills = %#v", app.skills)
	}
}
