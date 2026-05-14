package agent

import (
	"context"
	"errors"
	"path/filepath"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/memory"
	"github.com/pandelisz/gode/internal/godex/provider"
)

func TestRunnerInjectsPromptMemoryRecallBeforeUserPrompt(t *testing.T) {
	ctx := context.Background()
	mem := testMemoryService(t, memory.Config{Enabled: true, AutoRecall: true, RecallLimit: 3}, &testMemoryEmbedder{
		values: map[string][]float32{
			"prefer event bus plugins": {1, 0, 0},
			"event bus":                {1, 0, 0},
		},
	})
	saved, err := mem.Save(ctx, "prefer event bus plugins", "test")
	if err != nil {
		t.Fatalf("save memory: %v", err)
	}
	capture := &captureProvider{finalText: "done"}
	bus := eventbus.New(eventbus.WithSubscriberBuffer(16))
	defer bus.Close()
	runner := NewRunner(Config{Bus: bus, Provider: capture, Memory: mem})

	if _, err := runner.Run(ctx, RunRequest{SessionID: "s1", RunID: "r1", Prompt: "event bus"}); err != nil {
		t.Fatalf("run: %v", err)
	}
	if len(capture.request.Messages) < 2 {
		t.Fatalf("messages = %#v", capture.request.Messages)
	}
	recall := capture.request.Messages[len(capture.request.Messages)-2]
	user := capture.request.Messages[len(capture.request.Messages)-1]
	if recall.Role != provider.RoleUser || !strings.Contains(recall.Content, saved.ID) || !strings.Contains(recall.Content, "prefer event bus plugins") {
		t.Fatalf("recall message = %#v", recall)
	}
	if user.Content != "event bus" {
		t.Fatalf("user message = %#v", user)
	}
}

func TestRunnerSkipsMemoryRecallWhenDisabledOrAutoRecallOff(t *testing.T) {
	for _, tc := range []struct {
		name string
		cfg  memory.Config
	}{
		{name: "disabled", cfg: memory.Config{Enabled: false, AutoRecall: true}},
		{name: "auto off", cfg: memory.Config{Enabled: true, AutoRecall: false}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			capture := &captureProvider{finalText: "done"}
			runner := NewRunner(Config{
				Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
				Provider: capture,
				Memory:   testMemoryService(t, tc.cfg, &testMemoryEmbedder{}),
			})
			defer runner.bus.Close()
			if _, err := runner.Run(context.Background(), RunRequest{Prompt: "event bus"}); err != nil {
				t.Fatalf("run: %v", err)
			}
			if len(capture.request.Messages) != 1 || capture.request.Messages[0].Content != "event bus" {
				t.Fatalf("messages = %#v", capture.request.Messages)
			}
		})
	}
}

func TestRunnerMemoryRecallErrorEmitsNonFatalEvent(t *testing.T) {
	bus := eventbus.New(eventbus.WithSubscriberBuffer(16))
	defer bus.Close()
	events := bus.Subscribe(context.Background(), eventbus.Filter{Kinds: []eventbus.Kind{eventbus.KindMemoryRecallFailed}})
	capture := &captureProvider{finalText: "done"}
	store, scope := testMemoryStoreAndScope(t)
	good := memory.NewService(store, &testMemoryEmbedder{}, scope, memory.Config{Enabled: true, AutoRecall: true}, nil)
	if _, err := good.Save(context.Background(), "event bus memory", "test"); err != nil {
		t.Fatalf("save seed memory: %v", err)
	}
	runner := NewRunner(Config{
		Bus:      bus,
		Provider: capture,
		Memory: memory.NewService(store, &testMemoryEmbedder{
			err: errors.New("embedding offline"),
		}, scope, memory.Config{Enabled: true, AutoRecall: true}, nil),
	})

	result, err := runner.Run(context.Background(), RunRequest{SessionID: "s1", RunID: "r1", Prompt: "event bus"})
	if err != nil {
		t.Fatalf("run should continue after recall failure: %v", err)
	}
	if result.FinalText != "done" {
		t.Fatalf("final = %q", result.FinalText)
	}
	ev := <-events
	if ev.Kind != eventbus.KindMemoryRecallFailed {
		t.Fatalf("event = %#v", ev)
	}
	var payload struct {
		Query string `json:"query"`
		Error string `json:"error"`
	}
	if err := ev.DecodePayload(&payload); err != nil {
		t.Fatalf("decode: %v", err)
	}
	if payload.Query != "event bus" || !strings.Contains(payload.Error, "embedding offline") {
		t.Fatalf("payload = %#v", payload)
	}
}

type testMemoryEmbedder struct {
	values map[string][]float32
	err    error
}

func (e *testMemoryEmbedder) Model() string { return "embed" }

func (e *testMemoryEmbedder) Embed(_ context.Context, input string) (memory.Vector, error) {
	if e.err != nil {
		return memory.Vector{}, e.err
	}
	values := e.values[input]
	if len(values) == 0 {
		values = []float32{1, 0, 0}
	}
	return memory.Vector{Model: "embed", Dimensions: len(values), Values: append([]float32(nil), values...)}, nil
}

func testMemoryService(t *testing.T, cfg memory.Config, embedder memory.Embedder) *memory.Service {
	t.Helper()
	store, scope := testMemoryStoreAndScope(t)
	return memory.NewService(store, embedder, scope, cfg, nil)
}

func testMemoryStoreAndScope(t *testing.T) (*memory.Store, memory.Scope) {
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
	return store, scope
}
