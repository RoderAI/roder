package builtin

import (
	"context"
	"path/filepath"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex/memory"
	"github.com/pandelisz/gode/internal/godex/tools"
)

func TestTodoAndMemoryTools(t *testing.T) {
	reg := tools.NewRegistry()
	RegisterTodo(reg)
	RegisterMemory(reg, testMemoryService(t))

	todo, err := reg.Run(context.Background(), tools.Call{Name: "todo_update", Input: map[string]any{
		"items": []any{
			map[string]any{"content": "build bus", "status": "completed"},
			map[string]any{"content": "wire tui", "status": "pending"},
		},
	}})
	if err != nil {
		t.Fatalf("todo: %v", err)
	}
	if !strings.Contains(todo.Text, "[completed] build bus") || !strings.Contains(todo.Text, "[pending] wire tui") {
		t.Fatalf("todo text = %q", todo.Text)
	}

	if _, err := reg.Run(context.Background(), tools.Call{Name: "memory_save", Input: map[string]any{"content": "prefer event bus"}}); err != nil {
		t.Fatalf("memory save: %v", err)
	}
	mem, err := reg.Run(context.Background(), tools.Call{Name: "memory_find", Input: map[string]any{"query": "event bus"}})
	if err != nil {
		t.Fatalf("memory find: %v", err)
	}
	if !strings.Contains(mem.Text, "prefer event bus") {
		t.Fatalf("memory text = %q", mem.Text)
	}
}

type testEmbedder struct{}

func (testEmbedder) Model() string { return "embed" }

func (testEmbedder) Embed(context.Context, string) (memory.Vector, error) {
	return memory.Vector{Model: "embed", Dimensions: 3, Values: []float32{1, 0, 0}}, nil
}

func testMemoryService(t *testing.T) *memory.Service {
	t.Helper()
	store, err := memory.OpenStore(context.Background(), filepath.Join(t.TempDir(), "memories.sqlite3"))
	if err != nil {
		t.Fatalf("open store: %v", err)
	}
	t.Cleanup(func() { _ = store.Close() })
	scope, err := memory.NewScope(t.TempDir(), "", t.TempDir())
	if err != nil {
		t.Fatalf("scope: %v", err)
	}
	return memory.NewService(store, testEmbedder{}, scope, memory.Config{Enabled: true}, nil)
}
