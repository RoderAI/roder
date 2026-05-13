package appserver

import (
	"context"
	"encoding/json"
	"strings"
	"time"

	"github.com/pandelisz/gode/internal/godex/goals"
)

type threadGoalGetParams struct {
	ThreadID string `json:"threadId"`
}

type threadGoalSetParams struct {
	ThreadID    string `json:"threadId"`
	Objective   string `json:"objective,omitempty"`
	Status      string `json:"status,omitempty"`
	TokenBudget *int64 `json:"tokenBudget,omitempty"`
	Replace     *bool  `json:"replaceExisting,omitempty"`
}

func (s *Server) handleThreadGoalGet(ctx context.Context, raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[threadGoalGetParams](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	if strings.TrimSpace(params.ThreadID) == "" {
		return nil, rpcError(errorInvalidParams, "threadId is required")
	}
	goal, err := s.app.GetGoal(ctx, params.ThreadID)
	if err != nil {
		return nil, rpcError(errorInternal, err.Error())
	}
	return map[string]any{"goal": goalView(goal)}, nil
}

func (s *Server) handleThreadGoalSet(ctx context.Context, raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[threadGoalSetParams](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	if strings.TrimSpace(params.ThreadID) == "" {
		return nil, rpcError(errorInvalidParams, "threadId is required")
	}
	replace := true
	if params.Replace != nil {
		replace = *params.Replace
	}
	req := goals.SetRequest{
		SessionID:       params.ThreadID,
		Objective:       params.Objective,
		Status:          goals.GoalStatus(params.Status),
		TokenBudget:     params.TokenBudget,
		ReplaceExisting: replace,
	}
	goal, err := s.app.SetGoal(ctx, req)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	return map[string]any{"goal": goalView(goal)}, nil
}

func (s *Server) handleThreadGoalClear(ctx context.Context, raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[threadGoalGetParams](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	if strings.TrimSpace(params.ThreadID) == "" {
		return nil, rpcError(errorInvalidParams, "threadId is required")
	}
	if err := s.app.ClearGoal(ctx, params.ThreadID); err != nil {
		return nil, rpcError(errorInternal, err.Error())
	}
	return map[string]any{}, nil
}

func goalView(goal *goals.Goal) any {
	if goal == nil {
		return nil
	}
	out := map[string]any{
		"threadId":        goal.SessionID,
		"goalId":          goal.GoalID,
		"objective":       goal.Objective,
		"status":          goal.Status,
		"tokensUsed":      goal.TokensUsed,
		"timeUsedSeconds": goal.TimeUsedSeconds,
		"createdAt":       goal.CreatedAt.Format(time.RFC3339Nano),
		"updatedAt":       goal.UpdatedAt.Format(time.RFC3339Nano),
		"remainingTokens": goals.RemainingTokens(*goal),
	}
	if goal.TokenBudget != nil {
		out["tokenBudget"] = *goal.TokenBudget
	}
	return out
}
