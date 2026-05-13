package tui

import (
	"context"
	"strings"

	tea "charm.land/bubbletea/v2"
	"github.com/google/uuid"
	"github.com/pandelisz/gode/internal/godex/eventbus"
	godexgoals "github.com/pandelisz/gode/internal/godex/goals"
	tuigoals "github.com/pandelisz/gode/internal/tui/goals"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func (m *Model) handleGoalInput(prompt string) (bool, tea.Cmd) {
	cmd, ok, err := tuigoals.Parse(prompt)
	if !ok {
		return false, nil
	}
	if err != nil {
		m.addMessage(viewmodel.RoleError, "goal", err.Error())
		m.status = "goal failed - ctrl+l errors"
		return true, nil
	}
	if m.app == nil {
		m.addMessage(viewmodel.RoleError, "goal", "goal commands require an app")
		m.status = "goal failed - ctrl+l errors"
		return true, nil
	}
	sessionID := m.ensureGoalSessionID()
	ctx := context.Background()
	switch cmd.Action {
	case tuigoals.ActionShow:
		goal, err := m.app.GetGoal(ctx, sessionID)
		return m.finishGoalCommand(goal, err, "goal"), nil
	case tuigoals.ActionSet:
		goal, err := m.app.SetGoal(ctx, godexgoals.SetRequest{SessionID: sessionID, Objective: cmd.Objective, ReplaceExisting: true})
		if !m.finishGoalCommand(goal, err, "goal set") || err != nil {
			return true, nil
		}
		m.addMessage(viewmodel.RoleUser, "", cmd.Objective)
		m.reasoningSummary = ""
		m.attachments = nil
		m.running = true
		m.status = "waiting for model"
		return true, m.runPrompt(cmd.Objective)
	case tuigoals.ActionPause:
		goal, err := m.app.SetGoal(ctx, godexgoals.SetRequest{SessionID: sessionID, Status: godexgoals.StatusPaused})
		return m.finishGoalCommand(goal, err, "goal paused"), nil
	case tuigoals.ActionResume:
		goal, err := m.app.SetGoal(ctx, godexgoals.SetRequest{SessionID: sessionID, Status: godexgoals.StatusActive})
		return m.finishGoalCommand(goal, err, "goal resumed"), nil
	case tuigoals.ActionClear:
		err := m.app.ClearGoal(ctx, sessionID)
		if err != nil {
			return m.finishGoalCommand(nil, err, "goal"), nil
		}
		m.goalSummary = ""
		m.status = "goal cleared"
		m.addMessage(viewmodel.RoleSystem, "goal", "goal cleared")
		return true, nil
	case tuigoals.ActionBudget:
		budget := cmd.TokenBudget
		goal, err := m.app.SetGoal(ctx, godexgoals.SetRequest{SessionID: sessionID, TokenBudget: &budget})
		return m.finishGoalCommand(goal, err, "goal budget set"), nil
	default:
		m.addMessage(viewmodel.RoleError, "goal", "unsupported goal command")
		m.status = "goal failed - ctrl+l errors"
		return true, nil
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
	status := m.status
	if notice := m.copyNotice(); notice != "" {
		status = notice
	}
	if len(m.queuedPrompts) > 0 {
		queue := queueStatus(len(m.queuedPrompts))
		if strings.TrimSpace(status) == "" || status == "ready" {
			status = queue
		} else {
			status += " · " + queue
		}
	}
	if strings.TrimSpace(m.goalSummary) == "" {
		return status
	}
	if strings.TrimSpace(status) == "" || status == "ready" {
		return m.goalSummary
	}
	return status + " · " + m.goalSummary
}
