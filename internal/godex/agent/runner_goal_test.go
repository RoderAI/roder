package agent

import (
	"context"
	"path/filepath"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/goals"
	"github.com/pandelisz/gode/internal/godex/journal"
	"github.com/pandelisz/gode/internal/godex/provider"
)

func TestRunnerInjectsActiveGoalContext(t *testing.T) {
	goalRuntime := testGoalRuntime(t, nil, nil)
	if _, err := goalRuntime.Set(context.Background(), goals.SetRequest{SessionID: "s-goal", Objective: "ship the demo"}); err != nil {
		t.Fatalf("set goal: %v", err)
	}
	capture := &captureProvider{finalText: "done"}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Provider: capture,
		Goals:    goalRuntime,
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{SessionID: "s-goal", Prompt: "continue"}); err != nil {
		t.Fatalf("run: %v", err)
	}
	if len(capture.request.Messages) != 2 {
		t.Fatalf("messages = %#v", capture.request.Messages)
	}
	if capture.request.Messages[0].Role != provider.RoleSystem || !strings.Contains(capture.request.Messages[0].Content, "ship the demo") {
		t.Fatalf("goal context = %#v", capture.request.Messages[0])
	}
	if capture.request.Messages[1].Role != provider.RoleUser || capture.request.Messages[1].Content != "continue" {
		t.Fatalf("user prompt = %#v", capture.request.Messages[1])
	}
}

func TestRunnerSkipsPausedAndCompleteGoalContext(t *testing.T) {
	for _, status := range []goals.GoalStatus{goals.StatusPaused, goals.StatusComplete} {
		t.Run(string(status), func(t *testing.T) {
			goalRuntime := testGoalRuntime(t, nil, nil)
			if _, err := goalRuntime.Set(context.Background(), goals.SetRequest{SessionID: "s-goal", Objective: "ship"}); err != nil {
				t.Fatalf("set goal: %v", err)
			}
			if _, err := goalRuntime.Set(context.Background(), goals.SetRequest{SessionID: "s-goal", Status: status}); err != nil {
				t.Fatalf("set status: %v", err)
			}
			capture := &captureProvider{finalText: "done"}
			runner := NewRunner(Config{
				Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
				Provider: capture,
				Goals:    goalRuntime,
			})
			defer runner.bus.Close()

			if _, err := runner.Run(context.Background(), RunRequest{SessionID: "s-goal", Prompt: "continue"}); err != nil {
				t.Fatalf("run: %v", err)
			}
			if len(capture.request.Messages) != 1 || capture.request.Messages[0].Role != provider.RoleUser {
				t.Fatalf("messages = %#v", capture.request.Messages)
			}
		})
	}
}

func TestRunnerGoalUsageCanBudgetLimit(t *testing.T) {
	bus := eventbus.New(eventbus.WithSubscriberBuffer(16))
	defer bus.Close()
	store, err := journal.Open(filepath.Join(t.TempDir(), "events.jsonl"))
	if err != nil {
		t.Fatalf("journal: %v", err)
	}
	defer store.Close()
	goalRuntime := testGoalRuntime(t, bus, store)
	budget := int64(1)
	if _, err := goalRuntime.Set(context.Background(), goals.SetRequest{SessionID: "s-goal", Objective: "ship", TokenBudget: &budget}); err != nil {
		t.Fatalf("set goal: %v", err)
	}
	runner := NewRunner(Config{
		Bus:      bus,
		Journal:  store,
		Provider: &captureProvider{finalText: "done"},
		Goals:    goalRuntime,
	})

	if _, err := runner.Run(context.Background(), RunRequest{SessionID: "s-goal", Prompt: "continue"}); err != nil {
		t.Fatalf("run: %v", err)
	}
	goal, err := goalRuntime.Get(context.Background(), "s-goal")
	if err != nil {
		t.Fatalf("get goal: %v", err)
	}
	if goal == nil || goal.Status != goals.StatusBudgetLimited || goal.TokensUsed <= 0 {
		t.Fatalf("goal = %#v", goal)
	}
	events, err := store.Replay(context.Background(), journal.ReplayFilter{SessionID: "s-goal", Kinds: []eventbus.Kind{eventbus.KindGoalLimited}})
	if err != nil {
		t.Fatalf("replay: %v", err)
	}
	if len(events) == 0 {
		t.Fatalf("missing budget limited event")
	}
}

func testGoalRuntime(t *testing.T, bus *eventbus.Bus, journalStore *journal.Store) *goals.Runtime {
	t.Helper()
	store, err := goals.Open(t.TempDir())
	if err != nil {
		t.Fatalf("goal store: %v", err)
	}
	return goals.NewRuntime(store, bus, journalStore)
}
