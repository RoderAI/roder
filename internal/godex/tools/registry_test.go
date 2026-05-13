package tools

import (
	"context"
	"errors"
	"os"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/hooks"
	"github.com/pandelisz/gode/internal/godex/permission"
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

func TestRegistryUsesPermissionServiceWithMetadata(t *testing.T) {
	bus := eventbus.New(eventbus.WithSubscriberBuffer(8))
	defer bus.Close()
	service := permission.New(permission.WithEventBus(bus))
	reg := NewRegistry(WithEventBus(bus), WithAutoApprove(false), WithPermissionService(service))
	reg.Register(Tool{
		Name:        "write_file",
		Description: "write",
		Action:      permission.ActionWrite,
		PathFromInput: func(input map[string]any) string {
			path, _ := input["path"].(string)
			return path
		},
		Run: func(context.Context, Call) (Result, error) {
			return Result{Text: "wrote"}, nil
		},
	})
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	events := bus.Subscribe(ctx, eventbus.Filter{Kinds: []eventbus.Kind{eventbus.KindPermissionRequested}})

	errCh := make(chan error, 1)
	go func() {
		_, err := reg.Run(ctx, Call{Name: "write_file", SessionID: "s1", RunID: "r1", Input: map[string]any{"path": "README.md"}})
		errCh <- err
	}()

	req := readToolEvent(t, events)
	var payload struct {
		Tool   string `json:"tool"`
		Action string `json:"action"`
		Path   string `json:"path"`
	}
	if err := req.DecodePayload(&payload); err != nil {
		t.Fatalf("decode: %v", err)
	}
	if payload.Tool != "write_file" || payload.Action != string(permission.ActionWrite) || payload.Path != "README.md" {
		t.Fatalf("payload = %#v", payload)
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

func TestRegistryAllowedToolSkipsPermissionPrompt(t *testing.T) {
	bus := eventbus.New(eventbus.WithSubscriberBuffer(8))
	defer bus.Close()
	reg := NewRegistry(WithEventBus(bus), WithAutoApprove(false), WithAllowedTools("write_file"))
	reg.Register(Tool{
		Name: "write_file",
		Run: func(context.Context, Call) (Result, error) {
			return Result{Text: "wrote"}, nil
		},
	})
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	events := bus.Subscribe(ctx, eventbus.Filter{Kinds: []eventbus.Kind{eventbus.KindPermissionRequested}})

	result, err := reg.Run(context.Background(), Call{Name: "write_file", SessionID: "s1"})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.Text != "wrote" {
		t.Fatalf("result = %#v", result)
	}
	select {
	case ev := <-events:
		t.Fatalf("unexpected permission event: %#v", ev)
	default:
	}
}

func TestRegistryHookRewritesInputBeforePermissionAndRun(t *testing.T) {
	bus := eventbus.New(eventbus.WithSubscriberBuffer(8))
	defer bus.Close()
	service := permission.New(permission.WithEventBus(bus))
	runner := hooks.New([]hooks.Hook{{
		Name:    "rewrite",
		Command: hookScript(t, `printf '{"decision":"allow","updated_input":{"path":"rewritten.txt"}}'`),
		Tools:   []string{"write_file"},
	}})
	reg := NewRegistry(WithEventBus(bus), WithAutoApprove(false), WithPermissionService(service), WithHookRunner(runner), WithWorkspace(t.TempDir()))
	var gotPath string
	reg.Register(Tool{
		Name:        "write_file",
		Description: "write",
		Action:      permission.ActionWrite,
		PathFromInput: func(input map[string]any) string {
			path, _ := input["path"].(string)
			return path
		},
		Run: func(_ context.Context, call Call) (Result, error) {
			gotPath, _ = call.Input["path"].(string)
			return Result{Text: "wrote " + gotPath}, nil
		},
	})

	result, err := reg.Run(context.Background(), Call{Name: "write_file", SessionID: "s1", Input: map[string]any{"path": "original.txt"}})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if gotPath != "rewritten.txt" || result.Text != "wrote rewritten.txt" {
		t.Fatalf("gotPath=%q result=%#v", gotPath, result)
	}
}

func TestRegistryHookDenyReturnsFailedToolResult(t *testing.T) {
	bus := eventbus.New(eventbus.WithSubscriberBuffer(8))
	defer bus.Close()
	runner := hooks.New([]hooks.Hook{{
		Name:    "deny",
		Command: hookScript(t, `exit 2`),
		Tools:   []string{"write_file"},
	}})
	reg := NewRegistry(WithEventBus(bus), WithHookRunner(runner), WithWorkspace(t.TempDir()))
	reg.Register(Tool{
		Name: "write_file",
		Run: func(context.Context, Call) (Result, error) {
			t.Fatal("tool should not run after hook deny")
			return Result{}, nil
		},
	})

	result, err := reg.Run(context.Background(), Call{Name: "write_file", SessionID: "s1"})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.Error == "" || !strings.Contains(result.Text, "blocked by hook") {
		t.Fatalf("result = %#v", result)
	}
}

func hookScript(t *testing.T, body string) string {
	t.Helper()
	path := t.TempDir() + "/hook.sh"
	if err := os.WriteFile(path, []byte("#!/bin/sh\n"+body+"\n"), 0o700); err != nil {
		t.Fatalf("write hook: %v", err)
	}
	return path
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
