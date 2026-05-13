package permission

import (
	"context"
	"testing"
	"time"

	"github.com/pandelisz/gode/internal/godex/eventbus"
)

func TestServiceOneShotGrant(t *testing.T) {
	bus := eventbus.New(eventbus.WithSubscriberBuffer(8))
	defer bus.Close()
	service := New(WithEventBus(bus))
	events := subscribePermissions(t, bus)

	resultCh := make(chan authorizeResult, 1)
	go func() {
		result, err := service.Authorize(context.Background(), Request{
			SessionID: "s1",
			RunID:     "r1",
			Tool:      "write_file",
			Action:    ActionWrite,
			Path:      "README.md",
			Input:     map[string]any{"path": "README.md"},
		})
		resultCh <- authorizeResult{result: result, err: err}
	}()

	requested := nextEvent(t, events)
	if requested.Kind != eventbus.KindPermissionRequested {
		t.Fatalf("kind = %q", requested.Kind)
	}
	var payload struct {
		Tool   string         `json:"tool"`
		Action string         `json:"action"`
		Path   string         `json:"path"`
		Input  map[string]any `json:"input"`
	}
	if err := requested.DecodePayload(&payload); err != nil {
		t.Fatalf("decode: %v", err)
	}
	if payload.Tool != "write_file" || payload.Action != string(ActionWrite) || payload.Path != "README.md" || payload.Input["path"] != "README.md" {
		t.Fatalf("request payload = %#v", payload)
	}

	bus.Publish(context.Background(), eventbus.Event{
		Kind:          eventbus.KindPermissionResponded,
		Source:        eventbus.SourceTUI,
		SessionID:     "s1",
		RunID:         "r1",
		CorrelationID: requested.CorrelationID,
		Payload:       map[string]any{"approved": true},
	})
	got := waitAuthorize(t, resultCh)
	if !got.Approved || got.AllowForSession {
		t.Fatalf("result = %#v", got)
	}
}

func TestServicePersistentSessionGrant(t *testing.T) {
	bus := eventbus.New(eventbus.WithSubscriberBuffer(8))
	defer bus.Close()
	service := New(WithEventBus(bus))
	events := subscribePermissions(t, bus)
	req := Request{SessionID: "s1", Tool: "apply_patch", Action: ActionWrite, Path: "main.go"}

	resultCh := make(chan authorizeResult, 1)
	go func() {
		result, err := service.Authorize(context.Background(), req)
		resultCh <- authorizeResult{result: result, err: err}
	}()
	requested := nextEvent(t, events)
	bus.Publish(context.Background(), eventbus.Event{
		Kind:          eventbus.KindPermissionResponded,
		Source:        eventbus.SourceTUI,
		SessionID:     "s1",
		CorrelationID: requested.CorrelationID,
		Payload:       map[string]any{"approved": true, "allow_for_session": true},
	})
	first := waitAuthorize(t, resultCh)
	if !first.Approved || !first.AllowForSession {
		t.Fatalf("first = %#v", first)
	}

	second, err := service.Authorize(context.Background(), req)
	if err != nil {
		t.Fatalf("second authorize: %v", err)
	}
	if !second.Approved || second.Reason != "session_grant" {
		t.Fatalf("second = %#v", second)
	}
	assertNoEvent(t, events)
}

func TestServiceDeny(t *testing.T) {
	bus := eventbus.New(eventbus.WithSubscriberBuffer(8))
	defer bus.Close()
	service := New(WithEventBus(bus))
	events := subscribePermissions(t, bus)

	resultCh := make(chan authorizeResult, 1)
	go func() {
		result, err := service.Authorize(context.Background(), Request{SessionID: "s1", Tool: "shell", Action: ActionExecute})
		resultCh <- authorizeResult{result: result, err: err}
	}()
	requested := nextEvent(t, events)
	bus.Publish(context.Background(), eventbus.Event{
		Kind:          eventbus.KindPermissionResponded,
		Source:        eventbus.SourceTUI,
		SessionID:     "s1",
		CorrelationID: requested.CorrelationID,
		Payload:       map[string]any{"approved": false, "reason": "nope"},
	})
	got := waitAuthorize(t, resultCh)
	if got.Approved || got.Reason != "nope" {
		t.Fatalf("deny = %#v", got)
	}
}

func TestServiceAllowedToolsAndSkipRequests(t *testing.T) {
	bus := eventbus.New(eventbus.WithSubscriberBuffer(8))
	defer bus.Close()
	allowed := New(WithEventBus(bus), WithAllowedTools("read_file"))
	events := subscribePermissions(t, bus)
	result, err := allowed.Authorize(context.Background(), Request{SessionID: "s1", Tool: "read_file", Action: ActionRead})
	if err != nil {
		t.Fatalf("allowed authorize: %v", err)
	}
	if !result.Approved || result.Reason != "allowed_tool" {
		t.Fatalf("allowed result = %#v", result)
	}
	assertNoEvent(t, events)

	skip := New(WithEventBus(bus), WithSkipRequests(true))
	result, err = skip.Authorize(context.Background(), Request{SessionID: "s1", Tool: "shell", Action: ActionExecute})
	if err != nil {
		t.Fatalf("skip authorize: %v", err)
	}
	if !result.Approved || result.Reason != "skip_requests" {
		t.Fatalf("skip result = %#v", result)
	}
	assertNoEvent(t, events)
}

type authorizeResult struct {
	result Result
	err    error
}

func subscribePermissions(t *testing.T, bus *eventbus.Bus) <-chan eventbus.Event {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	t.Cleanup(cancel)
	return bus.Subscribe(ctx, eventbus.Filter{Kinds: []eventbus.Kind{eventbus.KindPermissionRequested}})
}

func nextEvent(t *testing.T, events <-chan eventbus.Event) eventbus.Event {
	t.Helper()
	select {
	case ev := <-events:
		return ev
	case <-time.After(time.Second):
		t.Fatal("timed out waiting for permission event")
		return eventbus.Event{}
	}
}

func waitAuthorize(t *testing.T, ch <-chan authorizeResult) Result {
	t.Helper()
	select {
	case got := <-ch:
		if got.err != nil {
			t.Fatalf("authorize: %v", got.err)
		}
		return got.result
	case <-time.After(time.Second):
		t.Fatal("timed out waiting for authorize")
		return Result{}
	}
}

func assertNoEvent(t *testing.T, events <-chan eventbus.Event) {
	t.Helper()
	select {
	case ev := <-events:
		t.Fatalf("unexpected permission event: %#v", ev)
	case <-time.After(20 * time.Millisecond):
	}
}
