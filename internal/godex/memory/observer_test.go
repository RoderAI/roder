package memory

import (
	"context"
	"errors"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex/provider"
)

func TestMemoryObserverIsInactiveWhenAutoObserveOff(t *testing.T) {
	observer := NewObserver(newTestService(t, Config{Enabled: true, AutoObserve: false}), &observerProvider{})
	if observer.Enabled() {
		t.Fatal("observer should be disabled when auto_observe is false")
	}
}

func TestMemoryObserverSendsBoundedContextAndOnlyMemoryTools(t *testing.T) {
	capture := &observerProvider{}
	observer := NewObserver(newTestService(t, Config{Enabled: true, AutoObserve: true}), capture)
	messages := make([]provider.Message, MaxObservationMessages+5)
	for i := range messages {
		messages[i] = provider.Message{Role: provider.RoleUser, Content: "message"}
	}

	if err := observer.Observe(context.Background(), ObservationRequest{SessionID: "s1", RunID: "r1", Messages: messages}); err != nil {
		t.Fatalf("observe: %v", err)
	}
	if len(capture.request.Messages) != MaxObservationMessages {
		t.Fatalf("messages len = %d", len(capture.request.Messages))
	}
	if len(capture.request.Tools) != 1 || capture.request.Tools[0].Name != "memory_save" {
		t.Fatalf("tools = %#v", capture.request.Tools)
	}
	if strings.Contains(capture.request.Instructions, "shell") {
		t.Fatalf("instructions should not mention shell tools: %s", capture.request.Instructions)
	}
}

func TestMemoryObserverSavesToolCallsWithObserverMetadata(t *testing.T) {
	service := newTestService(t, Config{Enabled: true, AutoObserve: true})
	capture := &observerProvider{events: []provider.Event{
		{Kind: provider.EventToolCall, ToolRequest: &provider.ToolRequest{
			ID:    "call_save",
			Name:  "memory_save",
			Input: map[string]any{"content": "Prefer compact files"},
		}},
		{Kind: provider.EventCompleted, Text: "done"},
	}}
	observer := NewObserver(service, capture)

	if err := observer.Observe(context.Background(), ObservationRequest{SessionID: "s1", RunID: "r1", Messages: []provider.Message{{Role: provider.RoleUser, Content: "context"}}}); err != nil {
		t.Fatalf("observe: %v", err)
	}
	entries, err := service.List(context.Background(), 10)
	if err != nil {
		t.Fatalf("list: %v", err)
	}
	if len(entries) != 1 || entries[0].Source != "observer" || entries[0].Metadata["session_id"] != "s1" || entries[0].Metadata["run_id"] != "r1" {
		t.Fatalf("entries = %#v", entries)
	}
}

func TestMemoryObserverReturnsSaveFailure(t *testing.T) {
	service := newTestServiceWithEmbedder(t, Config{Enabled: true, AutoObserve: true}, failingObserverEmbedder{})
	observer := NewObserver(service, &observerProvider{events: []provider.Event{
		{Kind: provider.EventToolCall, ToolRequest: &provider.ToolRequest{Name: "memory_save", Input: map[string]any{"content": "remember"}}},
	}})

	err := observer.Observe(context.Background(), ObservationRequest{SessionID: "s1", RunID: "r1"})
	if err == nil || !strings.Contains(err.Error(), "embed failed") {
		t.Fatalf("err = %v", err)
	}
}

type failingObserverEmbedder struct{}

func (failingObserverEmbedder) Model() string { return "embed" }

func (failingObserverEmbedder) Embed(context.Context, string) (Vector, error) {
	return Vector{}, errors.New("embed failed")
}

type observerProvider struct {
	request provider.Request
	events  []provider.Event
	err     error
}

func (p *observerProvider) Name() string { return "observer" }

func (p *observerProvider) Stream(_ context.Context, req provider.Request) (<-chan provider.Event, <-chan error) {
	p.request = req
	events := make(chan provider.Event, len(p.events))
	errs := make(chan error, 1)
	for _, ev := range p.events {
		events <- ev
	}
	if p.err != nil {
		errs <- p.err
	}
	close(events)
	close(errs)
	return events, errs
}
