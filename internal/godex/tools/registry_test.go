package tools

import (
	"context"
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
