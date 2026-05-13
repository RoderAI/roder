package builtin

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"

	"github.com/pandelisz/gode/internal/godex/goals"
	"github.com/pandelisz/gode/internal/godex/tools"
)

func RegisterGoal(reg *tools.Registry, runtime *goals.Runtime) {
	reg.Register(tools.Tool{
		Name:        "get_goal",
		Description: "Return the current session goal, usage, and budget state. Returns null when no goal exists.",
		ReadOnly:    true,
		Schema:      objectSchema(),
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			goal, err := runtime.Get(ctx, call.SessionID)
			if err != nil {
				return tools.Result{}, err
			}
			return goalResult(goal)
		},
	})
	reg.Register(tools.Tool{
		Name:        "create_goal",
		Description: "Create a goal only when the user or system/developer instructions explicitly requested long-running goal behavior. Pause, resume, clear, and budget-limit are user/runtime controlled.",
		ReadOnly:    false,
		Schema: map[string]any{
			"type": "object",
			"properties": map[string]any{
				"objective":    map[string]any{"type": "string"},
				"token_budget": map[string]any{"type": "integer"},
			},
			"required": []string{"objective"},
		},
		SkipPermission: true,
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			var budget *int64
			if value := intInputDefault(call.Input, "token_budget", 0); value > 0 {
				parsed := int64(value)
				budget = &parsed
			}
			goal, err := runtime.Set(ctx, goals.SetRequest{SessionID: call.SessionID, Objective: stringInput(call.Input, "objective"), TokenBudget: budget})
			if err != nil {
				return tools.Result{}, err
			}
			return goalResult(goal)
		},
	})
	reg.Register(tools.Tool{
		Name:        "update_goal",
		Description: "Mark the current goal complete after auditing that the objective is actually finished. Only {\"status\":\"complete\"} is allowed; pause, resume, clear, and budget-limit are not model-controlled.",
		ReadOnly:    false,
		Schema: map[string]any{
			"type": "object",
			"properties": map[string]any{
				"status": map[string]any{"type": "string", "enum": []string{string(goals.StatusComplete)}},
			},
			"required": []string{"status"},
		},
		SkipPermission: true,
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			if goals.GoalStatus(stringInput(call.Input, "status")) != goals.StatusComplete {
				return tools.Result{}, errors.New("update_goal only accepts status=complete")
			}
			goal, err := runtime.Set(ctx, goals.SetRequest{SessionID: call.SessionID, Status: goals.StatusComplete})
			if err != nil {
				return tools.Result{}, err
			}
			return goalResult(goal)
		},
	})
}

func goalResult(goal *goals.Goal) (tools.Result, error) {
	payload := map[string]any{
		"goal":                     goal,
		"remaining_tokens":         nil,
		"completion_budget_report": "",
	}
	if goal != nil {
		payload["remaining_tokens"] = goals.RemainingTokens(*goal)
		payload["completion_budget_report"] = goals.CompletionBudgetReport(*goal)
	}
	data, err := json.MarshalIndent(payload, "", "  ")
	if err != nil {
		return tools.Result{}, fmt.Errorf("encode goal result: %w", err)
	}
	return tools.Result{Text: string(data), Data: payload}, nil
}
