package builtin

import (
	"context"
	"path/filepath"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex/tools"
)

func TestTodoAndMemoryTools(t *testing.T) {
	reg := tools.NewRegistry()
	RegisterTodo(reg)
	RegisterMemory(reg, filepath.Join(t.TempDir(), "memory.jsonl"))

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

	if _, err := reg.Run(context.Background(), tools.Call{Name: "memory_add", Input: map[string]any{"note": "prefer event bus"}}); err != nil {
		t.Fatalf("memory add: %v", err)
	}
	mem, err := reg.Run(context.Background(), tools.Call{Name: "memory_list"})
	if err != nil {
		t.Fatalf("memory list: %v", err)
	}
	if !strings.Contains(mem.Text, "prefer event bus") {
		t.Fatalf("memory text = %q", mem.Text)
	}
}
