package goals

import (
	"context"
	"testing"

	"github.com/pandelisz/gode/internal/godex/eventbus"
)

func TestRuntimePublishesGoalEvents(t *testing.T) {
	bus := eventbus.New(eventbus.WithSubscriberBuffer(8))
	defer bus.Close()
	runtime := NewRuntime(openTestStore(t), bus, nil)
	events := bus.Subscribe(context.Background(), eventbus.Filter{SessionID: "s1"})

	goal, err := runtime.Set(context.Background(), SetRequest{SessionID: "s1", Objective: "ship"})
	if err != nil {
		t.Fatalf("set: %v", err)
	}
	if goal.Status != StatusActive {
		t.Fatalf("goal = %#v", goal)
	}
	ev := <-events
	if ev.Kind != eventbus.KindGoalUpdated {
		t.Fatalf("event = %#v", ev)
	}

	if err := runtime.Clear(context.Background(), "s1"); err != nil {
		t.Fatalf("clear: %v", err)
	}
	ev = <-events
	if ev.Kind != eventbus.KindGoalCleared {
		t.Fatalf("clear event = %#v", ev)
	}
}
