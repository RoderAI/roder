package goals

import "testing"

func TestParseGoalCommands(t *testing.T) {
	tests := []struct {
		input  string
		action Action
		value  string
		budget int64
	}{
		{input: "/goal", action: ActionShow},
		{input: "/goal ship the release", action: ActionSet, value: "ship the release"},
		{input: "/goal pause", action: ActionPause},
		{input: "/goal resume", action: ActionResume},
		{input: "/goal clear", action: ActionClear},
		{input: "/goal budget 50000", action: ActionBudget, budget: 50000},
	}
	for _, tt := range tests {
		cmd, ok, err := Parse(tt.input)
		if err != nil {
			t.Fatalf("%q err = %v", tt.input, err)
		}
		if !ok || cmd.Action != tt.action || cmd.Objective != tt.value || cmd.TokenBudget != tt.budget {
			t.Fatalf("%q = %#v ok=%v", tt.input, cmd, ok)
		}
	}
}

func TestParseGoalBudgetRejectsInvalidValue(t *testing.T) {
	if _, ok, err := Parse("/goal budget 0"); !ok || err == nil {
		t.Fatalf("expected invalid budget error, ok=%v err=%v", ok, err)
	}
}
