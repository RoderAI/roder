package tui

import (
	"context"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/godex"
	messagestore "github.com/pandelisz/gode/internal/godex/message"
	"github.com/pandelisz/gode/internal/godex/session"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func TestCompactCommandCompactsCurrentSession(t *testing.T) {
	ctx := context.Background()
	app, err := godex.New(ctx, godex.Config{DataDir: t.TempDir(), Workspace: t.TempDir(), Provider: "mock", AutoApprove: true})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	defer app.Close(ctx)
	sessionID := "s-compact"
	if _, err := app.Sessions.Ensure(ctx, session.Session{ID: sessionID, Title: "hello"}); err != nil {
		t.Fatalf("ensure session: %v", err)
	}
	if _, err := app.Messages.Append(ctx, messagestore.Message{SessionID: sessionID, Role: messagestore.RoleUser, Text: "hello"}); err != nil {
		t.Fatalf("append message: %v", err)
	}
	model := New(app)
	defer model.cancelEvents()
	model.currentSessionID = sessionID
	model.input.SetValue("/compact")

	updated, cmd := model.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got := updated.(Model)
	if cmd == nil {
		t.Fatal("compact command should return a command")
	}
	if got.input.Value() != "" {
		t.Fatalf("input = %q", got.input.Value())
	}
	if got.status != "compacting context" {
		t.Fatalf("status = %q", got.status)
	}
	updated, _ = got.Update(cmd())
	got = updated.(Model)
	if got.status != "context compacted" {
		t.Fatalf("status after compact = %q", got.status)
	}
	messages, err := app.Messages.ListBySession(ctx, sessionID)
	if err != nil {
		t.Fatalf("messages: %v", err)
	}
	if len(messages) != 2 {
		t.Fatalf("messages = %#v", messages)
	}
	last := messages[len(messages)-1]
	if last.Role != messagestore.RoleCompaction || !strings.Contains(string(last.RawJSON), `"id":"cmp_mock"`) {
		t.Fatalf("last message = %#v", last)
	}
}

func TestCompactCommandRequiresActiveSession(t *testing.T) {
	model := New(nil)
	model.input.SetValue("/compact")

	updated, cmd := model.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got := updated.(Model)
	if cmd != nil {
		t.Fatal("compact without app should not return a command")
	}
	if got.status != "compact failed - ctrl+l errors" {
		t.Fatalf("status = %q", got.status)
	}
	if len(got.messages) != 1 || got.messages[0].Role != viewmodel.RoleError || !strings.Contains(got.messages[0].Body, "requires an app") {
		t.Fatalf("messages = %#v", got.messages)
	}
}

func TestSlashMenuIncludesCompactCommand(t *testing.T) {
	model := New(nil)
	model.input.SetValue("/co")

	menu := model.slashMenuViewModel()
	if menu == nil || len(menu.Items) != 1 {
		t.Fatalf("slash menu = %#v", menu)
	}
	if menu.Items[0].ID != "compact" || menu.Items[0].Label != "/compact" || !strings.Contains(menu.Items[0].Description, "force compaction") {
		t.Fatalf("slash menu item = %#v", menu.Items[0])
	}
}
