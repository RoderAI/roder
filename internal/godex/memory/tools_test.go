package memory

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex/tools"
)

func TestRegisterToolsExposesMemoryToolSet(t *testing.T) {
	service := newTestService(t, Config{Enabled: true})
	reg := tools.NewRegistry()
	RegisterTools(reg, service)

	names := specNames(reg)
	for _, want := range []string{"memory_delete", "memory_find", "memory_read", "memory_save", "memory_update"} {
		if !names[want] {
			t.Fatalf("tool %q missing from specs %#v", want, names)
		}
	}
}

func TestMemoryToolsSaveQueryReadUpdateDelete(t *testing.T) {
	ctx := context.Background()
	embedder := &countingEmbedder{model: "embed", values: map[string][]float32{
		"prefer event bus":         {1, 0, 0},
		"prefer event bus plugins": {0, 1, 0},
		"event bus":                {1, 0, 0},
	}}
	service := newTestServiceWithEmbedder(t, Config{Enabled: true}, embedder)
	reg := tools.NewRegistry()
	RegisterTools(reg, service)

	saved, err := reg.Run(ctx, tools.Call{Name: "memory_save", Input: map[string]any{"content": " prefer   event bus "}})
	if err != nil {
		t.Fatalf("memory_save: %v", err)
	}
	savedData, ok := saved.Data.(ToolEntry)
	if !ok || savedData.ID == "" || savedData.Content != "prefer event bus" {
		t.Fatalf("saved data = %#v", saved.Data)
	}

	queried, err := reg.Run(ctx, tools.Call{Name: "memory_find", Input: map[string]any{"query": "event bus", "limit": 5}})
	if err != nil {
		t.Fatalf("memory_find: %v", err)
	}
	rows, ok := queried.Data.([]ToolEntry)
	if !ok || len(rows) != 1 {
		t.Fatalf("query data = %#v", queried.Data)
	}
	if rows[0].ID != savedData.ID || rows[0].Score == 0 || rows[0].UpdatedAt == "" {
		t.Fatalf("query row = %#v", rows[0])
	}
	var textRows []ToolEntry
	if err := json.Unmarshal([]byte(queried.Text), &textRows); err != nil {
		t.Fatalf("query text should be json: %v\n%s", err, queried.Text)
	}

	read, err := reg.Run(ctx, tools.Call{Name: "memory_read", Input: map[string]any{"id": savedData.ID}})
	if err != nil {
		t.Fatalf("memory_read: %v", err)
	}
	if read.Text != "prefer event bus" {
		t.Fatalf("read text = %q", read.Text)
	}

	updated, err := reg.Run(ctx, tools.Call{Name: "memory_update", Input: map[string]any{"id": savedData.ID, "content": "prefer event bus plugins"}})
	if err != nil {
		t.Fatalf("memory_update: %v", err)
	}
	if updated.Data.(ToolEntry).Content != "prefer event bus plugins" {
		t.Fatalf("updated data = %#v", updated.Data)
	}

	deleted, err := reg.Run(ctx, tools.Call{Name: "memory_delete", Input: map[string]any{"id": savedData.ID}})
	if err != nil {
		t.Fatalf("memory_delete: %v", err)
	}
	if !strings.Contains(deleted.Text, savedData.ID) {
		t.Fatalf("delete text = %q", deleted.Text)
	}
	if _, err := service.Read(ctx, savedData.ID); !errors.Is(err, ErrNotFound) {
		t.Fatalf("read deleted err = %v", err)
	}
}

func TestMemoryFindCanListSavedMemories(t *testing.T) {
	ctx := context.Background()
	service := newTestService(t, Config{Enabled: true})
	reg := tools.NewRegistry()
	RegisterTools(reg, service)

	added, err := reg.Run(ctx, tools.Call{Name: "memory_save", Input: map[string]any{"content": "prefer sqlite memories"}})
	if err != nil {
		t.Fatalf("memory_save: %v", err)
	}
	if added.Data.(ToolEntry).ID == "" {
		t.Fatalf("added data = %#v", added.Data)
	}
	listed, err := reg.Run(ctx, tools.Call{Name: "memory_find", Input: map[string]any{"query": "sqlite memories"}})
	if err != nil {
		t.Fatalf("memory_find: %v", err)
	}
	if !strings.Contains(listed.Text, "prefer sqlite memories") {
		t.Fatalf("memory_find text = %q", listed.Text)
	}
}

func TestReadMemoryRespectsWorkspaceScope(t *testing.T) {
	ctx := context.Background()
	store := openTestStore(t)
	defer store.Close()
	embedder := &countingEmbedder{model: "embed"}
	local := NewService(store, embedder, testScope(t, "repo-a"), Config{Enabled: true}, nil)
	other := NewService(store, embedder, testScope(t, "repo-b"), Config{Enabled: true}, nil)
	saved, err := local.Save(ctx, "local only memory", "test")
	if err != nil {
		t.Fatalf("save: %v", err)
	}
	reg := tools.NewRegistry()
	RegisterTools(reg, other)

	result, err := reg.Run(ctx, tools.Call{Name: "memory_read", Input: map[string]any{"id": saved.ID}})
	if err != nil {
		t.Fatalf("memory_read run: %v", err)
	}
	if result.Error == "" || !strings.Contains(result.Error, ErrNotFound.Error()) {
		t.Fatalf("result = %#v", result)
	}
}

func specNames(reg *tools.Registry) map[string]bool {
	names := map[string]bool{}
	for _, spec := range reg.Specs() {
		names[spec.Name] = true
	}
	return names
}
