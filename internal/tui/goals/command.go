package goals

import (
	"fmt"
	"strconv"
	"strings"
)

type Action string

const (
	ActionShow   Action = "show"
	ActionSet    Action = "set"
	ActionPause  Action = "pause"
	ActionResume Action = "resume"
	ActionClear  Action = "clear"
	ActionBudget Action = "budget"
)

type Command struct {
	Action      Action
	Objective   string
	TokenBudget int64
}

func Parse(input string) (Command, bool, error) {
	trimmed := strings.TrimSpace(input)
	if trimmed != "/goal" && !strings.HasPrefix(trimmed, "/goal ") {
		return Command{}, false, nil
	}
	rest := strings.TrimSpace(strings.TrimPrefix(trimmed, "/goal"))
	if rest == "" {
		return Command{Action: ActionShow}, true, nil
	}
	switch {
	case rest == "pause":
		return Command{Action: ActionPause}, true, nil
	case rest == "resume":
		return Command{Action: ActionResume}, true, nil
	case rest == "clear":
		return Command{Action: ActionClear}, true, nil
	case strings.HasPrefix(rest, "budget "):
		raw := strings.TrimSpace(strings.TrimPrefix(rest, "budget "))
		budget, err := strconv.ParseInt(raw, 10, 64)
		if err != nil || budget <= 0 {
			return Command{}, true, fmt.Errorf("goal budget must be a positive integer")
		}
		return Command{Action: ActionBudget, TokenBudget: budget}, true, nil
	default:
		return Command{Action: ActionSet, Objective: rest}, true, nil
	}
}
