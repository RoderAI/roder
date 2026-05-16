package roadmap

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/pandelisz/gode/internal/godex/eventbus"
)

func TestRuntimeOpenFocusValidateSetTaskAndThreads(t *testing.T) {
	ctx := context.Background()
	workspace := t.TempDir()
	dataDir := t.TempDir()
	path := filepath.Join(workspace, "roadmap", "30-test.md")
	writeFile(t, path, sampleRoadmap("first task"))
	doc, err := ParseFile(path)
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
		eventbus.KindRoadmapModeChanged,
	}})
	runtime := NewRuntime(workspace, dataDir, bus)

	summaries, err := runtime.ListDocuments(ctx)
	if err != nil {
		t.Fatalf("list: %v", err)
	}
	if len(summaries) != 1 || summaries[0].Unchecked != 1 {
		t.Fatalf("summaries = %#v", summaries)
	}
	opened, err := runtime.Open(ctx, "roadmap/30-test.md")
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	if opened.Title != "Test" {
		t.Fatalf("opened = %#v", opened)
	}
	if err := runtime.FocusTask(ctx, path, doc.Tasks[0].ID); err != nil {
		t.Fatalf("focus: %v", err)
	}
	if err := runtime.SetTask(ctx, path, doc.Tasks[0].ID, true, "test passed"); err != nil {
		t.Fatalf("set task: %v", err)
	}
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read updated: %v", err)
	}
	if string(raw) == sampleRoadmap("first task") {
		t.Fatal("task was not updated")
	}
	validation, err := runtime.Validate(ctx, path)
	if err != nil {
		t.Fatalf("validate: %v", err)
	}
	if len(validation.Diagnostics) != 0 {
		t.Fatalf("validation = %#v", validation)
	}
	spawned, err := runtime.SpawnThread(ctx, path, doc.Tasks[0].ID)
	if err != nil {
		t.Fatalf("spawn: %v", err)
	}
	attached, err := runtime.AttachThread(ctx, path, doc.Tasks[0].ID, "thread-known")
	if err != nil {
		t.Fatalf("attach: %v", err)
	}
	threads, err := runtime.ListThreads(ctx, path)
	if err != nil {
		t.Fatalf("threads: %v", err)
	}
	if len(threads) != 2 || threads[0].ThreadID != spawned.ThreadID || threads[1].ThreadID != attached.ThreadID {
		t.Fatalf("threads = %#v", threads)
	}
	skillPath := filepath.Join(workspace, planningSkillPath)
	writeFile(t, skillPath, "Roadmap skill body")
	prompt, err := runtime.ContextPrompt(ctx, path)
	if err != nil {
		t.Fatalf("context prompt: %v", err)
	}
	for _, want := range []string{"Roadmap: Test", "Focused task: first task", "Roadmap skill body"} {
		if !strings.Contains(prompt, want) {
			t.Fatalf("context prompt missing %q:\n%s", want, prompt)
		}
	}
	runtime.ModeChanged(ctx, true)

	seen := map[eventbus.Kind]bool{}
	deadline := time.After(2 * time.Second)
	for len(seen) < 8 {
		select {
		case ev := <-events:
			seen[ev.Kind] = true
		case <-deadline:
			t.Fatalf("missing roadmap events, saw %#v", seen)
		}
	}
}
