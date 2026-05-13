package memory

import (
	"context"
	"errors"
	"reflect"
	"testing"

	"github.com/pandelisz/gode/internal/godex/eventbus"
)

type countingEmbedder struct {
	model  string
	values map[string][]float32
	calls  []string
}

func (e *countingEmbedder) Model() string {
	return e.model
}

func (e *countingEmbedder) Embed(ctx context.Context, input string) (Vector, error) {
	e.calls = append(e.calls, input)
	values := e.values[input]
	if len(values) == 0 {
		values = []float32{1, 0, 0}
	}
	return Vector{Model: e.model, Dimensions: len(values), Values: append([]float32(nil), values...)}, nil
}

func TestServiceDisabledReturnsErrDisabled(t *testing.T) {
	ctx := context.Background()
	service := newTestService(t, Config{Enabled: false})

	if _, err := service.Save(ctx, "remember this", "tool"); !errors.Is(err, ErrDisabled) {
		t.Fatalf("save err = %v", err)
	}
	if _, err := service.Update(ctx, "mem_1", "new"); !errors.Is(err, ErrDisabled) {
		t.Fatalf("update err = %v", err)
	}
	if err := service.Delete(ctx, "mem_1"); !errors.Is(err, ErrDisabled) {
		t.Fatalf("delete err = %v", err)
	}
	if _, err := service.Query(ctx, "remember", 5); !errors.Is(err, ErrDisabled) {
		t.Fatalf("query err = %v", err)
	}
	if _, err := service.Read(ctx, "mem_1"); !errors.Is(err, ErrDisabled) {
		t.Fatalf("read err = %v", err)
	}
}

func TestServiceSaveUpdateDeleteRead(t *testing.T) {
	ctx := context.Background()
	embedder := &countingEmbedder{model: "embed", values: map[string][]float32{
		"prefer event bus":      {1, 0, 0},
		"prefer local sqlite":   {0, 1, 0},
		"prefer local memories": {0, 0, 1},
	}}
	service := newTestServiceWithEmbedder(t, Config{Enabled: true}, embedder)

	saved, err := service.Save(ctx, " prefer   event bus\n", "tool")
	if err != nil {
		t.Fatalf("save: %v", err)
	}
	if saved.ID == "" || saved.Content != "prefer event bus" || saved.Source != "tool" {
		t.Fatalf("saved = %#v", saved)
	}
	if !reflect.DeepEqual(embedder.calls, []string{"prefer event bus"}) {
		t.Fatalf("embed calls = %#v", embedder.calls)
	}
	read, err := service.Read(ctx, saved.ID)
	if err != nil {
		t.Fatalf("read: %v", err)
	}
	if read.Content != saved.Content {
		t.Fatalf("read = %#v", read)
	}

	updated, err := service.Update(ctx, saved.ID, "prefer local sqlite")
	if err != nil {
		t.Fatalf("update: %v", err)
	}
	if updated.Content != "prefer local sqlite" || updated.UpdatedAt.Before(saved.UpdatedAt) {
		t.Fatalf("updated = %#v", updated)
	}
	if !reflect.DeepEqual(embedder.calls, []string{"prefer event bus", "prefer local sqlite"}) {
		t.Fatalf("embed calls after update = %#v", embedder.calls)
	}

	if err := service.Delete(ctx, saved.ID); err != nil {
		t.Fatalf("delete: %v", err)
	}
	if _, err := service.Read(ctx, saved.ID); !errors.Is(err, ErrNotFound) {
		t.Fatalf("read deleted err = %v", err)
	}
}

func TestServiceQueryRanksAndCapsBySimilarity(t *testing.T) {
	ctx := context.Background()
	embedder := &countingEmbedder{model: "embed", values: map[string][]float32{
		"alpha": {1, 0, 0},
		"beta":  {0, 1, 0},
		"gamma": {0, 0, 1},
		"query": {0, 1, 0},
	}}
	service := newTestServiceWithEmbedder(t, Config{Enabled: true, RecallLimit: 2}, embedder)
	for _, content := range []string{"alpha", "beta", "gamma"} {
		if _, err := service.Save(ctx, content, "test"); err != nil {
			t.Fatalf("save %q: %v", content, err)
		}
	}

	results, err := service.Query(ctx, "query", 99)
	if err != nil {
		t.Fatalf("query: %v", err)
	}
	if len(results) != 3 {
		t.Fatalf("results len = %d, results = %#v", len(results), results)
	}
	if results[0].Content != "beta" || results[0].Score < results[1].Score {
		t.Fatalf("ranked results = %#v", results)
	}
	if got := embedder.calls[len(embedder.calls)-1]; got != "query" {
		t.Fatalf("query embed call = %q", got)
	}
}

func TestServiceQueryCapsLimitAboveMaximum(t *testing.T) {
	ctx := context.Background()
	embedder := &countingEmbedder{model: "embed", values: map[string][]float32{"query": {1, 0, 0}}}
	service := newTestServiceWithEmbedder(t, Config{Enabled: true}, embedder)
	for i := 0; i < MaxRecallLimit+5; i++ {
		content := "memory cap item " + string(rune('a'+i))
		embedder.values[content] = []float32{1, 0, 0}
		if _, err := service.Save(ctx, content, "test"); err != nil {
			t.Fatalf("save %q: %v", content, err)
		}
	}

	results, err := service.Query(ctx, "query", MaxRecallLimit+100)
	if err != nil {
		t.Fatalf("query: %v", err)
	}
	if len(results) != MaxRecallLimit {
		t.Fatalf("results len = %d, want %d", len(results), MaxRecallLimit)
	}
}

func TestServiceQueryOnlyUsesActiveWorkspace(t *testing.T) {
	ctx := context.Background()
	store := openTestStore(t)
	defer store.Close()
	embedder := &countingEmbedder{model: "embed", values: map[string][]float32{
		"local":  {1, 0, 0},
		"other":  {1, 0, 0},
		"search": {1, 0, 0},
	}}
	scope := testScope(t, "repo-a")
	other := testScope(t, "repo-b")
	service := NewService(store, embedder, scope, Config{Enabled: true}, nil)
	otherService := NewService(store, embedder, other, Config{Enabled: true}, nil)
	if _, err := service.Save(ctx, "local", "test"); err != nil {
		t.Fatalf("save local: %v", err)
	}
	if _, err := otherService.Save(ctx, "other", "test"); err != nil {
		t.Fatalf("save other: %v", err)
	}

	results, err := service.Query(ctx, "search", 10)
	if err != nil {
		t.Fatalf("query: %v", err)
	}
	if len(results) != 1 || results[0].Content != "local" {
		t.Fatalf("workspace results = %#v", results)
	}
}

func TestServiceEmitsMemoryEvents(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	bus := eventbus.New()
	events := bus.Subscribe(ctx, eventbus.Filter{Kinds: []eventbus.Kind{KindMemorySaved, KindMemoryQueried}})
	service := newTestServiceWithBus(t, Config{Enabled: true}, &countingEmbedder{model: "embed"}, bus)

	if _, err := service.Save(ctx, "remember", "tool"); err != nil {
		t.Fatalf("save: %v", err)
	}
	if _, err := service.Query(ctx, "remember", 1); err != nil {
		t.Fatalf("query: %v", err)
	}

	first := <-events
	if first.Kind != KindMemorySaved {
		t.Fatalf("first event = %#v", first)
	}
	second := <-events
	if second.Kind != KindMemoryQueried {
		t.Fatalf("second event = %#v", second)
	}
}

func newTestService(t *testing.T, cfg Config) *Service {
	t.Helper()
	return newTestServiceWithEmbedder(t, cfg, &countingEmbedder{model: "embed"})
}

func newTestServiceWithEmbedder(t *testing.T, cfg Config, embedder Embedder) *Service {
	t.Helper()
	return newTestServiceWithBus(t, cfg, embedder, nil)
}

func newTestServiceWithBus(t *testing.T, cfg Config, embedder Embedder, bus *eventbus.Bus) *Service {
	t.Helper()
	store := openTestStore(t)
	t.Cleanup(func() { _ = store.Close() })
	scope := testScope(t, "repo")
	return NewService(store, embedder, scope, cfg, bus)
}
