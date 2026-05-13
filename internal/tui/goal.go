package tui

import (
	"context"
	"strings"

	"github.com/google/uuid"
	"github.com/pandelisz/gode/internal/godex/eventbus"
	godexgoals "github.com/pandelisz/gode/internal/godex/goals"
	tuigoals "github.com/pandelisz/gode/internal/tui/goals"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func (m *Model) handleGoalInput(prompt string) bool {
	cmd, ok, err := tuigoals.Parse(prompt)
	if !ok {
		return false
	}
	if err != nil {
		m.addMessage(viewmodel.RoleError, "goal", err.Error())
		m.status = "goal failed - ctrl+l errors"
		return true
	}
	if m.app == nil {
		m.addMessage(viewmodel.RoleError, "goal", "goal commands require an app")
		m.status = "goal failed - ctrl+l errors"
		return true
	}
	sessionID := m.ensureGoalSessionID()
	ctx := context.Background()
	switch cmd.Action {
	case tuigoals.ActionShow:
		goal, err := m.app.GetGoal(ctx, sessionID)
		return m.finishGoalCommand(goal, err, "goal")
	case tuigoals.ActionSet:
		goal, err := m.app.SetGoal(ctx, godexgoals.SetRequest{SessionID: sessionID, Objective: cmd.Objective, ReplaceExisting: true})
		return m.finishGoalCommand(goal, err, "goal set")
	case tuigoals.ActionPause:
		goal, err := m.app.SetGoal(ctx, godexgoals.SetRequest{SessionID: sessionID, Status: godexgoals.StatusPaused})
		return m.finishGoalCommand(goal, err, "goal paused")
	case tuigoals.ActionResume:
		goal, err := m.app.SetGoal(ctx, godexgoals.SetRequest{SessionID: sessionID, Status: godexgoals.StatusActive})
		return m.finishGoalCommand(goal, err, "goal resumed")
	case tuigoals.ActionClear:
		err := m.app.ClearGoal(ctx, sessionID)
		if err != nil {
			return m.finishGoalCommand(nil, err, "goal")
		}
		m.goalSummary = ""
		m.status = "goal cleared"
		m.addMessage(viewmodel.RoleSystem, "goal", "goal cleared")
		return true
	case tuigoals.ActionBudget:
		budget := cmd.TokenBudget
		goal, err := m.app.SetGoal(ctx, godexgoals.SetRequest{SessionID: sessionID, TokenBudget: &budget})
		return m.finishGoalCommand(goal, err, "goal budget set")
	default:
		m.addMessage(viewmodel.RoleError, "goal", "unsupported goal command")
		m.status = "goal failed - ctrl+l errors"
		return true
	}
}

func (m *Model) finishGoalCommand(goal *godexgoals.Goal, err error, okStatus string) bool {
	if err != nil {
		m.addMessage(viewmodel.RoleError, "goal", err.Error())
		m.status = "goal failed - ctrl+l errors"
		return true
	}
	summary := tuigoals.Summary(goal)
	if goal != nil {
		m.goalSummary = summary
	}
	m.status = okStatus
	m.addMessage(viewmodel.RoleSystem, "goal", summary)
	return true
}

func (m *Model) ensureGoalSessionID() string {
	if strings.TrimSpace(m.currentSessionID) == "" {
		m.currentSessionID = uuid.NewString()
	}
	return m.currentSessionID
}

func (m *Model) applyGoalEvent(ev eventbus.Event) {
	switch ev.Kind {
	case eventbus.KindGoalUpdated, eventbus.KindGoalLimited:
		var goal godexgoals.Goal
		if err := ev.DecodePayload(&goal); err == nil && goal.GoalID != "" {
			m.goalSummary = tuigoals.Summary(&goal)
			return
		}
	case eventbus.KindGoalCleared:
		m.goalSummary = ""
	}
}

func (m Model) footerStatus() string {
	if strings.TrimSpace(m.goalSummary) == "" {
		return m.status
	}
	if strings.TrimSpace(m.status) == "" || m.status == "ready" {
		return m.goalSummary
	}
	return m.status + " · " + m.goalSummary
}
