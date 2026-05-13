package goals

import (
	"fmt"
	"strings"
	"time"

	godexgoals "github.com/pandelisz/gode/internal/godex/goals"
)

func Summary(goal *godexgoals.Goal) string {
	if goal == nil {
		return "no active goal"
	}
	prefix := fmt.Sprintf("goal %s", goal.Status)
	objective := strings.Join(strings.Fields(goal.Objective), " ")
	if len(objective) > 42 {
		objective = objective[:42] + "..."
	}
	parts := []string{prefix}
	if objective != "" {
		parts = append(parts, objective)
	}
	if elapsed := Elapsed(goal.TimeUsedSeconds); elapsed != "" {
		parts = append(parts, elapsed)
	}
	if budget := Budget(goal.TokensUsed, goal.TokenBudget); budget != "" {
		parts = append(parts, budget)
	}
	return strings.Join(parts, " · ")
}

func Elapsed(seconds int64) string {
	if seconds < 0 {
		seconds = 0
	}
	d := time.Duration(seconds) * time.Second
	if d < time.Minute {
		return fmt.Sprintf("%ds", seconds)
	}
	if d < time.Hour {
		return fmt.Sprintf("%dm", int64(d/time.Minute))
	}
	if d < 24*time.Hour {
		hours := int64(d / time.Hour)
		minutes := int64((d % time.Hour) / time.Minute)
		return fmt.Sprintf("%dh %dm", hours, minutes)
	}
	days := int64(d / (24 * time.Hour))
	hours := int64((d % (24 * time.Hour)) / time.Hour)
	minutes := int64((d % time.Hour) / time.Minute)
	return fmt.Sprintf("%dd %dh %dm", days, hours, minutes)
}

func Budget(used int64, budget *int64) string {
	if budget == nil {
		return ""
	}
	return compactNumber(used) + "/" + compactNumber(*budget)
}

func compactNumber(value int64) string {
	sign := ""
	if value < 0 {
		sign = "-"
		value = -value
	}
	switch {
	case value >= 1_000_000:
		if value%1_000_000 == 0 {
			return fmt.Sprintf("%s%dM", sign, value/1_000_000)
		}
		return fmt.Sprintf("%s%.1fM", sign, float64(value)/1_000_000)
	case value >= 1_000:
		if value%1_000 == 0 {
			return fmt.Sprintf("%s%dK", sign, value/1_000)
		}
		return fmt.Sprintf("%s%.1fK", sign, float64(value)/1_000)
	default:
		return fmt.Sprintf("%s%d", sign, value)
	}
}
