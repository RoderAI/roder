package godex

import (
	"context"
	"path/filepath"
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

	if _, err := app.RunPrompt(context.Background(), "hello"); err != nil {
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
}
