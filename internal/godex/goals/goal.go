package goals

import (
	"errors"
	"strings"
	"time"
	"unicode/utf8"
)

type GoalStatus string

const (
	StatusActive        GoalStatus = "active"
	StatusPaused        GoalStatus = "paused"
	StatusBudgetLimited GoalStatus = "budget_limited"
	StatusComplete      GoalStatus = "complete"
)

var (
	ErrNotFound          = errors.New("goal not found")
	ErrActiveGoalExists  = errors.New("active goal already exists")
	ErrInvalidStatus     = errors.New("invalid goal status")
	ErrInvalidObjective  = errors.New("objective is required")
	ErrObjectiveTooLarge = errors.New("objective must be at most 4000 characters")
	ErrInvalidBudget     = errors.New("token budget must be positive")
)

type Goal struct {
	SessionID       string     `json:"session_id"`
	GoalID          string     `json:"goal_id"`
	Objective       string     `json:"objective"`
	Status          GoalStatus `json:"status"`
	TokenBudget     *int64     `json:"token_budget,omitempty"`
	TokensUsed      int64      `json:"tokens_used"`
	TimeUsedSeconds int64      `json:"time_used_seconds"`
	CreatedAt       time.Time  `json:"created_at"`
	UpdatedAt       time.Time  `json:"updated_at"`
}

type SetRequest struct {
	SessionID       string
	Objective       string
	Status          GoalStatus
	TokenBudget     *int64
	ReplaceExisting bool
}

func ValidateObjective(objective string) (string, error) {
	objective = strings.TrimSpace(objective)
	if objective == "" {
		return "", ErrInvalidObjective
	}
	if utf8.RuneCountInString(objective) > 4000 {
		return "", ErrObjectiveTooLarge
	}
	return objective, nil
}

func ValidateBudget(budget *int64) error {
	if budget != nil && *budget <= 0 {
		return ErrInvalidBudget
	}
	return nil
}

func ValidStatus(status GoalStatus) bool {
	switch status {
	case StatusActive, StatusPaused, StatusBudgetLimited, StatusComplete:
		return true
	default:
		return false
	}
}

func IsContinuable(status GoalStatus) bool {
	return status == StatusActive
}
