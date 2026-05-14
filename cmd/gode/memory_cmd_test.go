package main

import (
	"context"
	"path/filepath"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/memory"
)

func TestRunMemoryListPrintsCurrentWorkspaceMemories(t *testing.T) {
	ctx := context.Background()
	useCLITestMemoryEmbedder(t)
	root := t.TempDir()
	dataDir := filepath.Join(root, "data")
	workspace := filepath.Join(root, "repo")
	otherWorkspace := filepath.Join(root, "other")
	saveCLITestMemory(t, ctx, dataDir, workspace, "prefer event bus memory commands")
	saveCLITestMemory(t, ctx, dataDir, otherWorkspace, "other workspace secret")

	out := captureStdout(t, func() error {
		return runMemory(ctx, []string{"list", "--workspace", workspace, "--data-dir", dataDir})
	})

	if !strings.Contains(out, "mem_") || !strings.Contains(out, "prefer event bus memory commands") {
		t.Fatalf("list output:\n%s", out)
	}
	if !strings.Contains(out, "updated_at") {
		t.Fatalf("list output should include updated_at header:\n%s", out)
	}
	if strings.Contains(out, "other workspace secret") {
		t.Fatalf("list crossed workspace scope:\n%s", out)
	}
}

func TestRunMemoryQuerySearchesCurrentWorkspace(t *testing.T) {
	ctx := context.Background()
	useCLITestMemoryEmbedder(t)
	root := t.TempDir()
	dataDir := filepath.Join(root, "data")
	workspace := filepath.Join(root, "repo")
	otherWorkspace := filepath.Join(root, "other")
	saveCLITestMemory(t, ctx, dataDir, workspace, "event bus plugins can subscribe to memory")
	saveCLITestMemory(t, ctx, dataDir, otherWorkspace, "other workspace should not appear")

	out := captureStdout(t, func() error {
		return runMemory(ctx, []string{"query", "--workspace", workspace, "--data-dir", dataDir, "event bus"})
	})

	if !strings.Contains(out, "mem_") || !strings.Contains(out, "event bus plugins") {
		t.Fatalf("query output:\n%s", out)
	}
	if !strings.Contains(out, "score") {
		t.Fatalf("query output should include score header:\n%s", out)
	}
	if strings.Contains(out, "other workspace should not appear") {
		t.Fatalf("query crossed workspace scope:\n%s", out)
	}
}

func TestRunMemoryEnableDisableWritesSettings(t *testing.T) {
	ctx := context.Background()
	root := t.TempDir()
	dataDir := filepath.Join(root, "data")
	workspace := filepath.Join(root, "repo")

	disableOut := captureStdout(t, func() error {
		return runMemory(ctx, []string{"disable", "--workspace", workspace, "--data-dir", dataDir})
	})
	if !strings.Contains(disableOut, "memories\tdisabled") {
		t.Fatalf("disable output:\n%s", disableOut)
	}
	settings, err := godex.LoadSettings(dataDir)
	if err != nil {
		t.Fatalf("load settings after disable: %v", err)
	}
	if settings.Memories.Enabled == nil || *settings.Memories.Enabled {
		t.Fatalf("settings memories enabled after disable = %#v", settings.Memories.Enabled)
	}

	enableOut := captureStdout(t, func() error {
		return run(context.Background(), []string{"memory", "enable", "--workspace", workspace, "--data-dir", dataDir})
	})
	if !strings.Contains(enableOut, "memories\tenabled") {
		t.Fatalf("enable output:\n%s", enableOut)
	}
	settings, err = godex.LoadSettings(dataDir)
	if err != nil {
		t.Fatalf("load settings after enable: %v", err)
	}
	if settings.Memories.Enabled == nil || !*settings.Memories.Enabled {
		t.Fatalf("settings memories enabled after enable = %#v", settings.Memories.Enabled)
	}
}

func saveCLITestMemory(t *testing.T, ctx context.Context, dataDir string, workspace string, content string) {
	t.Helper()
	scope, err := memory.NewScope(workspace, "", dataDir)
	if err != nil {
		t.Fatalf("scope: %v", err)
	}
	store, err := memory.OpenStore(ctx, scope.DatabasePath)
	if err != nil {
		t.Fatalf("open memory store: %v", err)
	}
	t.Cleanup(func() { _ = store.Close() })
	if err := store.UpsertWorkspace(ctx, scope); err != nil {
		t.Fatalf("upsert workspace: %v", err)
	}
	if _, err := store.Save(ctx, memory.Entry{
		WorkspaceID:   scope.WorkspaceID,
		WorkspaceRoot: scope.WorkspaceRoot,
		Content:       content,
		Source:        "test",
	}, memory.Vector{Model: memory.DefaultEmbeddingModel, Dimensions: 3, Values: []float32{1, 0, 0}}); err != nil {
		t.Fatalf("save memory: %v", err)
	}
}

func useCLITestMemoryEmbedder(t *testing.T) {
	t.Helper()
	previous := memoryCommandEmbedderFactory
	memoryCommandEmbedderFactory = func(model string) memory.Embedder {
		return cliTestMemoryEmbedder{model: model}
	}
	t.Cleanup(func() {
		memoryCommandEmbedderFactory = previous
	})
}

type cliTestMemoryEmbedder struct {
	model string
}

func (e cliTestMemoryEmbedder) Model() string {
	if e.model == "" {
		return memory.DefaultEmbeddingModel
	}
	return e.model
}

func (e cliTestMemoryEmbedder) Embed(context.Context, string) (memory.Vector, error) {
	return memory.Vector{Model: e.Model(), Dimensions: 3, Values: []float32{1, 0, 0}}, nil
}
