package agent

import (
	"context"
	"errors"
	"sync/atomic"
	"testing"
	"time"

	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/memory"
	"github.com/pandelisz/gode/internal/godex/provider"
	"github.com/pandelisz/gode/internal/godex/tools"
)

func TestRunnerMemoryObserverInactiveWhenAutoObserveFalse(t *testing.T) {
	observerProvider := &countingObserverProvider{called: make(chan struct{}, 1)}
	runner := NewRunner(Config{
		Bus:            eventbus.New(eventbus.WithSubscriberBuffer(32)),
		Tools:          observerTestTools(),
		Provider:       observerMainProvider(15),
		MemoryObserver: memory.NewObserver(testMemoryService(t, memory.Config{Enabled: true, AutoObserve: false}, &testMemoryEmbedder{}), observerProvider),
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{SessionID: "s1", RunID: "r1", Prompt: "go"}); err != nil {
		t.Fatalf("run: %v", err)
	}
	if observerProvider.Count() != 0 {
		t.Fatalf("observer calls = %d", observerProvider.Count())
	}
}

func TestRunnerStartsMemoryObserverAfterFifteenToolResults(t *testing.T) {
	observerProvider := &countingObserverProvider{called: make(chan struct{}, 1)}
	runner := NewRunner(Config{
		Bus:            eventbus.New(eventbus.WithSubscriberBuffer(32)),
		Tools:          observerTestTools(),
		Provider:       observerMainProvider(15),
		MemoryObserver: memory.NewObserver(testMemoryService(t, memory.Config{Enabled: true, AutoObserve: true}, &testMemoryEmbedder{}), observerProvider),
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{SessionID: "s1", RunID: "r1", Prompt: "go"}); err != nil {
		t.Fatalf("run: %v", err)
	}
	waitObserverCall(t, observerProvider.called)
	if observerProvider.Count() != 1 {
		t.Fatalf("observer calls = %d", observerProvider.Count())
	}
}

func TestRunnerStartsOnlyOneMemoryObserverPerRun(t *testing.T) {
	observerProvider := &countingObserverProvider{called: make(chan struct{}, 2)}
	runner := NewRunner(Config{
		Bus:   eventbus.New(eventbus.WithSubscriberBuffer(32)),
		Tools: observerTestTools(),
		Provider: &scriptedProvider{streams: [][]provider.Event{
			observerToolEvents(15),
			observerToolEvents(1),
			{{Kind: provider.EventDelta, Text: "done"}, {Kind: provider.EventCompleted, Text: "done"}},
		}},
		MemoryObserver: memory.NewObserver(testMemoryService(t, memory.Config{Enabled: true, AutoObserve: true}, &testMemoryEmbedder{}), observerProvider),
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{SessionID: "s1", RunID: "r1", Prompt: "go"}); err != nil {
		t.Fatalf("run: %v", err)
	}
	waitObserverCall(t, observerProvider.called)
	if observerProvider.Count() != 1 {
		t.Fatalf("observer calls = %d", observerProvider.Count())
	}
}

func TestRunnerMemoryObserverFailureEmitsEventAndDoesNotFailRun(t *testing.T) {
	bus := eventbus.New(eventbus.WithSubscriberBuffer(32))
	defer bus.Close()
	events := bus.Subscribe(context.Background(), eventbus.Filter{Kinds: []eventbus.Kind{eventbus.KindMemoryObserverFailed}})
	observerProvider := &countingObserverProvider{
		called: make(chan struct{}, 1),
		events: []provider.Event{{
			Kind:        provider.EventToolCall,
			ToolRequest: &provider.ToolRequest{Name: "memory_save", Input: map[string]any{"content": "remember"}},
		}},
	}
	runner := NewRunner(Config{
		Bus:            bus,
		Tools:          observerTestTools(),
		Provider:       observerMainProvider(15),
		MemoryObserver: memory.NewObserver(testMemoryService(t, memory.Config{Enabled: true, AutoObserve: true}, &testMemoryEmbedder{err: errors.New("embed failed")}), observerProvider),
	})

	result, err := runner.Run(context.Background(), RunRequest{SessionID: "s1", RunID: "r1", Prompt: "go"})
	if err != nil {
		t.Fatalf("run should not fail on observer failure: %v", err)
	}
	if result.FinalText != "done" {
		t.Fatalf("final = %q", result.FinalText)
	}
	waitObserverCall(t, observerProvider.called)
	select {
	case ev := <-events:
		var payload struct {
			Error string `json:"error"`
		}
		if err := ev.DecodePayload(&payload); err != nil {
			t.Fatalf("decode: %v", err)
		}
		if payload.Error == "" {
			t.Fatalf("payload = %#v", payload)
		}
	case <-time.After(time.Second):
		t.Fatal("timed out waiting for observer failure event")
	}
}

func TestRunnerMemoryObserverDoesNotBlockMainRun(t *testing.T) {
	observerProvider := &blockingObserverProvider{called: make(chan struct{}), release: make(chan struct{})}
	runner := NewRunner(Config{
		Bus:            eventbus.New(eventbus.WithSubscriberBuffer(32)),
		Tools:          observerTestTools(),
		Provider:       observerMainProvider(15),
		MemoryObserver: memory.NewObserver(testMemoryService(t, memory.Config{Enabled: true, AutoObserve: true}, &testMemoryEmbedder{}), observerProvider),
	})
	defer runner.bus.Close()

	done := make(chan error, 1)
	go func() {
		result, err := runner.Run(context.Background(), RunRequest{SessionID: "s1", RunID: "r1", Prompt: "go"})
		if err == nil && result.FinalText != "done" {
			err = errors.New("unexpected final text")
		}
		done <- err
	}()
	waitObserverCall(t, observerProvider.called)
	select {
	case err := <-done:
		if err != nil {
			t.Fatalf("run: %v", err)
		}
	case <-time.After(time.Second):
		t.Fatal("run blocked on observer")
	}
	close(observerProvider.release)
}

func observerTestTools() *tools.Registry {
	reg := tools.NewRegistry(tools.WithAutoApprove(true))
	reg.Register(tools.Tool{
		Name:     "noop",
		ReadOnly: true,
		Run: func(context.Context, tools.Call) (tools.Result, error) {
			return tools.Result{Text: "ok"}, nil
		},
	})
	return reg
}

func observerMainProvider(toolCalls int) *scriptedProvider {
	return &scriptedProvider{streams: [][]provider.Event{
		observerToolEvents(toolCalls),
		{{Kind: provider.EventDelta, Text: "done"}, {Kind: provider.EventCompleted, Text: "done"}},
	}}
}

func observerToolEvents(count int) []provider.Event {
	events := make([]provider.Event, 0, count+1)
	for i := 0; i < count; i++ {
		events = append(events, provider.Event{
			Kind:        provider.EventToolCall,
			ToolRequest: &provider.ToolRequest{ID: "call", Name: "noop", Input: map[string]any{}},
		})
	}
	events = append(events, provider.Event{Kind: provider.EventCompleted})
	return events
}

func waitObserverCall(t *testing.T, ch <-chan struct{}) {
	t.Helper()
	select {
	case <-ch:
	case <-time.After(time.Second):
		t.Fatal("timed out waiting for observer call")
	}
}

type countingObserverProvider struct {
	count  atomic.Int32
	called chan struct{}
	events []provider.Event
}

func (p *countingObserverProvider) Count() int {
	return int(p.count.Load())
}

func (p *countingObserverProvider) Name() string { return "observer" }

func (p *countingObserverProvider) Stream(context.Context, provider.Request) (<-chan provider.Event, <-chan error) {
	p.count.Add(1)
	if p.called != nil {
		select {
		case p.called <- struct{}{}:
		default:
		}
	}
	events := make(chan provider.Event, len(p.events))
	errs := make(chan error)
	for _, ev := range p.events {
		events <- ev
	}
	close(events)
	close(errs)
	return events, errs
}

type blockingObserverProvider struct {
	called  chan struct{}
	release chan struct{}
}

func (p *blockingObserverProvider) Name() string { return "observer" }

func (p *blockingObserverProvider) Stream(context.Context, provider.Request) (<-chan provider.Event, <-chan error) {
	close(p.called)
	<-p.release
	events := make(chan provider.Event)
	errs := make(chan error)
	close(events)
	close(errs)
	return events, errs
}
