package builtin

import (
	"context"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex/goals"
	"github.com/pandelisz/gode/internal/godex/tools"
)

func TestGoalToolsGetCreateAndComplete(t *testing.T) {
	reg, runtime := goalToolRegistry(t)

	empty, err := reg.Run(context.Background(), tools.Call{Name: "get_goal", SessionID: "s1"})
	if err != nil {
		t.Fatalf("get empty: %v", err)
	}
	if !strings.Contains(empty.Text, `"goal": null`) {
		t.Fatalf("empty text = %s", empty.Text)
	}

	created, err := reg.Run(context.Background(), tools.Call{Name: "create_goal", SessionID: "s1", Input: map[string]any{"objective": "ship it", "token_budget": 100}})
	if err != nil {
		t.Fatalf("create: %v", err)
	}
	if !strings.Contains(created.Text, `"objective": "ship it"`) || !strings.Contains(created.Text, `"remaining_tokens": 100`) {
		t.Fatalf("created text = %s", created.Text)
	}
	if _, err := runtime.Get(context.Background(), "s1"); err != nil {
		t.Fatalf("runtime get: %v", err)
	}

	paused, err := reg.Run(context.Background(), tools.Call{Name: "update_goal", SessionID: "s1", Input: map[string]any{"status": "paused"}})
	if err != nil {
		t.Fatalf("update invalid should return tool result: %v", err)
	}
	if paused.Error == "" || !strings.Contains(paused.Text, "only accepts status=complete") {
		t.Fatalf("paused result = %#v", paused)
	}

	completed, err := reg.Run(context.Background(), tools.Call{Name: "update_goal", SessionID: "s1", Input: map[string]any{"status": "complete"}})
	if err != nil {
		t.Fatalf("complete: %v", err)
	}
	if !strings.Contains(completed.Text, `"status": "complete"`) || !strings.Contains(completed.Text, "Goal used 0 of 100 tokens") {
		t.Fatalf("completed text = %s", completed.Text)
	}
}

func TestCreateGoalRejectsExistingActiveGoal(t *testing.T) {
	reg, _ := goalToolRegistry(t)
	if _, err := reg.Run(context.Background(), tools.Call{Name: "create_goal", SessionID: "s1", Input: map[string]any{"objective": "first"}}); err != nil {
		t.Fatalf("first: %v", err)
	}
	second, err := reg.Run(context.Background(), tools.Call{Name: "create_goal", SessionID: "s1", Input: map[string]any{"objective": "second"}})
	if err != nil {
		t.Fatalf("second should be tool result: %v", err)
	}
	if second.Error == "" || !strings.Contains(second.Text, "active goal already exists") {
		t.Fatalf("second = %#v", second)
	}
}

func TestGoalMutatingToolsSkipPermissionPrompts(t *testing.T) {
	store, err := goals.Open(t.TempDir())
	if err != nil {
		t.Fatalf("goals store: %v", err)
	}
	reg := tools.NewRegistry(tools.WithAutoApprove(false))
	RegisterGoal(reg, goals.NewRuntime(store, nil, nil))
	result, err := reg.Run(context.Background(), tools.Call{Name: "create_goal", SessionID: "s1", Input: map[string]any{"objective": "ship"}})
	if err != nil {
		t.Fatalf("create should not require permission bus: %v", err)
	}
	if result.Error != "" {
		t.Fatalf("create result = %#v", result)
	}
}

func goalToolRegistry(t *testing.T) (*tools.Registry, *goals.Runtime) {
	t.Helper()
	store, err := goals.Open(t.TempDir())
	if err != nil {
		t.Fatalf("goals store: %v", err)
	}
	runtime := goals.NewRuntime(store, nil, nil)
	reg := tools.NewRegistry()
	RegisterGoal(reg, runtime)
	return reg, runtime
}
