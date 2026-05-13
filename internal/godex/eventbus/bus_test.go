package eventbus

import (
	"context"
	"testing"
	"time"
)

func TestBusPublishesOrderedEventsToSubscribers(t *testing.T) {
	bus := New(WithSubscriberBuffer(4))
	defer bus.Close()

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	ch := bus.Subscribe(ctx, Filter{})
	first := Event{Kind: KindRunStarted, Source: SourceAgent, SessionID: "s1"}
	second := Event{Kind: KindAssistantDelta, Source: SourceProvider, SessionID: "s1"}

	bus.Publish(context.Background(), first)
	bus.Publish(context.Background(), second)

	gotFirst := readEvent(t, ch)
	gotSecond := readEvent(t, ch)

	if gotFirst.Seq != 1 || gotSecond.Seq != 2 {
		t.Fatalf("seqs = %d, %d; want 1, 2", gotFirst.Seq, gotSecond.Seq)
	}
	if gotFirst.ID == "" || gotSecond.ID == "" {
		t.Fatal("expected event ids to be assigned")
	}
	if gotFirst.Kind != KindRunStarted || gotSecond.Kind != KindAssistantDelta {
		t.Fatalf("kinds = %q, %q", gotFirst.Kind, gotSecond.Kind)
	}
}

func TestBusFiltersByKindAndSession(t *testing.T) {
	bus := New(WithSubscriberBuffer(2))
	defer bus.Close()

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	ch := bus.Subscribe(ctx, Filter{SessionID: "s2", Kinds: []Kind{KindToolCompleted}})
	bus.Publish(context.Background(), Event{Kind: KindToolCompleted, SessionID: "s1"})
	bus.Publish(context.Background(), Event{Kind: KindAssistantDelta, SessionID: "s2"})
	bus.Publish(context.Background(), Event{Kind: KindToolCompleted, SessionID: "s2"})

	got := readEvent(t, ch)
	if got.SessionID != "s2" || got.Kind != KindToolCompleted {
		t.Fatalf("got session=%q kind=%q", got.SessionID, got.Kind)
	}
	assertNoEvent(t, ch)
}

func TestBusAwaitReturnsCorrelatedEvent(t *testing.T) {
	bus := New(WithSubscriberBuffer(2))
	defer bus.Close()

	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()

	result := make(chan Event, 1)
	go func() {
		ev, err := bus.Await(ctx, Filter{CorrelationID: "c1", Kinds: []Kind{KindPermissionResponded}})
		if err != nil {
			t.Errorf("await: %v", err)
			return
		}
		result <- ev
	}()

	bus.Publish(context.Background(), Event{Kind: KindPermissionRequested, CorrelationID: "c1"})
	bus.Publish(context.Background(), Event{Kind: KindPermissionResponded, CorrelationID: "c1"})

	select {
	case ev := <-result:
		if ev.Kind != KindPermissionResponded {
			t.Fatalf("kind = %q", ev.Kind)
		}
	case <-ctx.Done():
		t.Fatal("timed out waiting for correlated event")
	}
}

func readEvent(t *testing.T, ch <-chan Event) Event {
	t.Helper()
	select {
	case ev := <-ch:
		return ev
	case <-time.After(time.Second):
		t.Fatal("timed out waiting for event")
		return Event{}
	}
}

func assertNoEvent(t *testing.T, ch <-chan Event) {
	t.Helper()
	select {
	case ev := <-ch:
		t.Fatalf("unexpected event: %+v", ev)
	case <-time.After(25 * time.Millisecond):
	}
}
