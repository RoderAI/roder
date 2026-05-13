package goals

import (
	"context"
	"errors"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestValidateObjectiveAndBudget(t *testing.T) {
	if got, err := ValidateObjective("  ship it  "); err != nil || got != "ship it" {
		t.Fatalf("validate objective = %q, %v", got, err)
	}
	if _, err := ValidateObjective(" "); !errors.Is(err, ErrInvalidObjective) {
		t.Fatalf("empty objective err = %v", err)
	}
	if _, err := ValidateObjective(strings.Repeat("x", 4001)); !errors.Is(err, ErrObjectiveTooLarge) {
		t.Fatalf("large objective err = %v", err)
	}
	bad := int64(0)
	if err := ValidateBudget(&bad); !errors.Is(err, ErrInvalidBudget) {
		t.Fatalf("bad budget err = %v", err)
	}
}

func TestStoreCreateGetSetClearAndReopen(t *testing.T) {
	store := openTestStore(t)
	budget := int64(100)
	goal, err := store.Set(context.Background(), SetRequest{SessionID: "s1", Objective: "ship it", TokenBudget: &budget})
	if err != nil {
		t.Fatalf("set: %v", err)
	}
	if goal.GoalID == "" || goal.Status != StatusActive || goal.TokenBudget == nil || *goal.TokenBudget != 100 {
		t.Fatalf("goal = %#v", goal)
	}
	got, err := store.Get(context.Background(), "s1")
	if err != nil {
		t.Fatalf("get: %v", err)
	}
	if got == nil || got.Objective != "ship it" {
		t.Fatalf("got = %#v", got)
	}
	paused, err := store.Set(context.Background(), SetRequest{SessionID: "s1", Status: StatusPaused})
	if err != nil {
		t.Fatalf("pause: %v", err)
	}
	if paused.GoalID != goal.GoalID || paused.Status != StatusPaused {
		t.Fatalf("paused = %#v", paused)
	}
	reopened, err := Open(storeRoot(store))
	if err != nil {
		t.Fatalf("reopen: %v", err)
	}
	reopenedGoal, err := reopened.Get(context.Background(), "s1")
	if err != nil {
		t.Fatalf("reopened get: %v", err)
	}
	if reopenedGoal == nil || reopenedGoal.Status != StatusPaused {
		t.Fatalf("reopened goal = %#v", reopenedGoal)
	}
	if err := reopened.Clear(context.Background(), "s1"); err != nil {
		t.Fatalf("clear: %v", err)
	}
	cleared, err := reopened.Get(context.Background(), "s1")
	if err != nil {
		t.Fatalf("cleared get: %v", err)
	}
	if cleared != nil {
		t.Fatalf("goal should be cleared: %#v", cleared)
	}
}

func TestStoreRejectsSecondActiveGoalWithoutReplace(t *testing.T) {
	store := openTestStore(t)
	if _, err := store.Set(context.Background(), SetRequest{SessionID: "s1", Objective: "first"}); err != nil {
		t.Fatalf("first: %v", err)
	}
	if _, err := store.Set(context.Background(), SetRequest{SessionID: "s1", Objective: "second"}); !errors.Is(err, ErrActiveGoalExists) {
		t.Fatalf("second err = %v", err)
	}
	replaced, err := store.Set(context.Background(), SetRequest{SessionID: "s1", Objective: "second", ReplaceExisting: true})
	if err != nil {
		t.Fatalf("replace: %v", err)
	}
	if replaced.Objective != "second" {
		t.Fatalf("replaced = %#v", replaced)
	}
}

func TestStoreAddUsageBudgetLimits(t *testing.T) {
	store := openTestStore(t)
	budget := int64(10)
	if _, err := store.Set(context.Background(), SetRequest{SessionID: "s1", Objective: "ship", TokenBudget: &budget}); err != nil {
		t.Fatalf("set: %v", err)
	}
	goal, err := store.AddUsage(context.Background(), "s1", 11, 2*time.Second)
	if err != nil {
		t.Fatalf("usage: %v", err)
	}
	if goal.Status != StatusBudgetLimited || goal.TokensUsed != 11 || goal.TimeUsedSeconds != 2 {
		t.Fatalf("goal = %#v", goal)
	}
}

func openTestStore(t *testing.T) *Store {
	t.Helper()
	store, err := Open(t.TempDir())
	if err != nil {
		t.Fatalf("open store: %v", err)
	}
	now := time.Date(2026, 5, 13, 10, 0, 0, 0, time.UTC)
	store.now = func() time.Time {
		now = now.Add(time.Second)
		return now
	}
	return store
}

func storeRoot(store *Store) string {
	return filepath.Dir(store.dir)
}
