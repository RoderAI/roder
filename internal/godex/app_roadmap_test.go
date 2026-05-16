package godex

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/roadmap"
)

func TestAppRoadmapMethodsDelegateToRuntime(t *testing.T) {
	ctx := context.Background()
	workspace := t.TempDir()
	dataDir := t.TempDir()
	path := filepath.Join(workspace, "roadmap", "30-test.md")
	writeRoadmapTestFile(t, path, "# Test\n\n**Goal:** Ship it.\n\n**Architecture:** Parse lines.\n\n**Tech Stack:** Go\n\n## Owned Paths\n\n- Create: `x`\n\n## Tasks\n\n- [ ] first task\n\nRun:\n\n```sh\ngo test ./...\n```\n\nAcceptance:\n- Works.\n")
	writeRoadmapTestFile(t, filepath.Join(workspace, ".agents", "skills", "roadmap-planning", "SKILL.md"), "Prefer delegable roadmap tasks.")

	doc, err := roadmap.ParseFile(path)
	if err != nil {
		t.Fatalf("parse fixture: %v", err)
	}
	bus := eventbus.New(eventbus.WithSubscriberBuffer(16))
	defer bus.Close()
	events := bus.Subscribe(ctx, eventbus.Filter{Kinds: []eventbus.Kind{
		eventbus.KindRoadmapOpened,
		eventbus.KindRoadmapTaskFocused,
		eventbus.KindRoadmapTaskChanged,
		eventbus.KindRoadmapUpdated,
		eventbus.KindRoadmapValidated,
		eventbus.KindRoadmapThreadSpawned,
		eventbus.KindRoadmapThreadAttached,
	}})
	app := &App{Roadmaps: roadmap.NewRuntime(workspace, dataDir, bus)}

	summaries, err := app.ListRoadmaps(ctx)
	if err != nil {
		t.Fatalf("list: %v", err)
	}
	if len(summaries) != 1 || summaries[0].Path != "roadmap/30-test.md" {
		t.Fatalf("summaries = %#v", summaries)
	}
	opened, err := app.OpenRoadmap(ctx, "roadmap/30-test.md")
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	if opened.Title != "Test" {
		t.Fatalf("opened = %#v", opened)
	}
	if err := app.FocusRoadmapTask(ctx, path, doc.Tasks[0].ID); err != nil {
		t.Fatalf("focus: %v", err)
	}
	if err := app.SetRoadmapTask(ctx, path, doc.Tasks[0].ID, true, "app test passed"); err != nil {
		t.Fatalf("set: %v", err)
	}
	validation, err := app.ValidateRoadmap(ctx, path)
	if err != nil {
		t.Fatalf("validate: %v", err)
	}
	if len(validation.Diagnostics) != 0 {
		t.Fatalf("validation = %#v", validation)
	}
	spawned, err := app.SpawnRoadmapThread(ctx, path, doc.Tasks[0].ID)
	if err != nil {
		t.Fatalf("spawn: %v", err)
	}
	attached, err := app.AttachRoadmapThread(ctx, path, doc.Tasks[0].ID, "thread-known")
	if err != nil {
		t.Fatalf("attach: %v", err)
	}
	threads, err := app.ListRoadmapThreads(ctx, path)
	if err != nil {
		t.Fatalf("threads: %v", err)
	}
	if len(threads) != 2 || threads[0].ThreadID != spawned.ThreadID || threads[1].ThreadID != attached.ThreadID {
		t.Fatalf("threads = %#v", threads)
	}
	prompt, err := app.RoadmapContextPrompt(ctx, path)
	if err != nil {
		t.Fatalf("context prompt: %v", err)
	}
	for _, want := range []string{"Roadmap: Test", "Focused task: first task", "Roadmap planning skill:"} {
		if !strings.Contains(prompt, want) {
			t.Fatalf("prompt missing %q:\n%s", want, prompt)
		}
	}

	seen := map[eventbus.Kind]bool{}
	deadline := time.After(2 * time.Second)
	for len(seen) < 7 {
		select {
		case ev := <-events:
			seen[ev.Kind] = true
		case <-deadline:
			t.Fatalf("missing roadmap events, saw %#v", seen)
		}
	}
}

func writeRoadmapTestFile(t *testing.T, path string, data string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatalf("mkdir %s: %v", path, err)
	}
	if err := os.WriteFile(path, []byte(data), 0o644); err != nil {
		t.Fatalf("write %s: %v", path, err)
	}
}
