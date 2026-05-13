package tools

import (
	"context"
	"errors"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex/eventbus"
)

func TestRegistryExecutesReadOnlyTool(t *testing.T) {
	reg := NewRegistry()
	reg.Register(Tool{
		Name:        "echo",
		Description: "echo input",
		ReadOnly:    true,
		Run: func(context.Context, Call) (Result, error) {
			return Result{Text: "ok"}, nil
		},
	})

	result, err := reg.Run(context.Background(), Call{Name: "echo"})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.Text != "ok" {
		t.Fatalf("text = %q", result.Text)
	}
}

func TestRegistryReturnsToolFailureAsResult(t *testing.T) {
	reg := NewRegistry()
	reg.Register(Tool{
		Name:        "apply_patch",
		Description: "patch",
		ReadOnly:    false,
		Run: func(context.Context, Call) (Result, error) {
			return Result{Text: "error: corrupt patch at line 4"}, errors.New("failed to apply patch: exit status 128")
		},
	})

	result, err := reg.Run(context.Background(), Call{Name: "apply_patch"})
	if err != nil {
		t.Fatalf("run should feed tool failure back as a result: %v", err)
	}
	if result.Error != "failed to apply patch: exit status 128" {
		t.Fatalf("error = %q", result.Error)
	}
	for _, want := range []string{"failed to apply patch: exit status 128", "error: corrupt patch at line 4"} {
		if !strings.Contains(result.Text, want) {
			t.Fatalf("result text missing %q:\n%s", want, result.Text)
		}
	}
}

func TestRegistryPublishesCompletedToolInput(t *testing.T) {
	bus := eventbus.New(eventbus.WithSubscriberBuffer(8))
	defer bus.Close()

	reg := NewRegistry(WithEventBus(bus))
	reg.Register(Tool{
		Name:        "read_file",
		Description: "read",
		ReadOnly:    true,
		Run: func(context.Context, Call) (Result, error) {
			return Result{Text: "contents"}, nil
		},
	})

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	events := bus.Subscribe(ctx, eventbus.Filter{Kinds: []eventbus.Kind{eventbus.KindToolCompleted}})

	if _, err := reg.Run(context.Background(), Call{Name: "read_file", Input: map[string]any{"path": "README.md"}}); err != nil {
		t.Fatalf("run: %v", err)
	}
	ev := <-events
	var payload struct {
		Tool  string         `json:"tool"`
		Input map[string]any `json:"input"`
		Text  string         `json:"text"`
	}
	if err := ev.DecodePayload(&payload); err != nil {
		t.Fatalf("decode: %v", err)
	}
	if payload.Tool != "read_file" || payload.Text != "contents" || payload.Input["path"] != "README.md" {
		t.Fatalf("payload = %#v", payload)
	}
}

func TestRegistryRequestsPermissionForMutatingTool(t *testing.T) {
	bus := eventbus.New(eventbus.WithSubscriberBuffer(4))
	defer bus.Close()

	reg := NewRegistry(WithEventBus(bus), WithAutoApprove(false))
	reg.Register(Tool{
		Name:        "write",
		Description: "write",
		ReadOnly:    false,
		Run: func(context.Context, Call) (Result, error) {
			return Result{Text: "wrote"}, nil
		},
	})

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	events := bus.Subscribe(ctx, eventbus.Filter{})

	errCh := make(chan error, 1)
	go func() {
		_, err := reg.Run(ctx, Call{Name: "write", SessionID: "s1", RunID: "r1"})
		errCh <- err
	}()

	req := readToolEvent(t, events)
	if req.Kind != eventbus.KindPermissionRequested {
		t.Fatalf("kind = %q", req.Kind)
	}
	bus.Publish(context.Background(), eventbus.Event{
		Kind:          eventbus.KindPermissionResponded,
		Source:        eventbus.SourceTUI,
		SessionID:     "s1",
		RunID:         "r1",
		CorrelationID: req.CorrelationID,
		Payload:       map[string]any{"approved": true},
	})

	if err := <-errCh; err != nil {
		t.Fatalf("run: %v", err)
	}
}

func readToolEvent(t *testing.T, ch <-chan eventbus.Event) eventbus.Event {
	t.Helper()
	for {
		select {
		case ev := <-ch:
			if ev.Kind == eventbus.KindPermissionRequested {
				return ev
			}
		case <-context.Background().Done():
		}
	}
}
