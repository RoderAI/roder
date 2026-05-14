package agent

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"sync"

	"github.com/google/uuid"
	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/provider"
	"github.com/pandelisz/gode/internal/godex/session"
)

var (
	ErrNoActiveRun       = errors.New("no active run to steer")
	ErrActiveRunMismatch = errors.New("active run mismatch")
)

type SteerRequest struct {
	SessionID     string
	Prompt        string
	ExpectedRunID string
}

type activeRun struct {
	sessionID       string
	runID           string
	mu              sync.Mutex
	steers          []string
	observerStarted bool
}

func (r *Runner) registerActiveRun(req RunRequest) *activeRun {
	control := &activeRun{sessionID: req.SessionID, runID: req.RunID}
	r.activeMu.Lock()
	if r.activeRuns == nil {
		r.activeRuns = map[string]*activeRun{}
	}
	r.activeRuns[req.SessionID] = control
	r.activeMu.Unlock()
	return control
}

func (r *Runner) unregisterActiveRun(control *activeRun) {
	if control == nil {
		return
	}
	r.activeMu.Lock()
	if r.activeRuns[control.sessionID] == control {
		delete(r.activeRuns, control.sessionID)
	}
	r.activeMu.Unlock()
}

func (r *Runner) Steer(ctx context.Context, req SteerRequest) (string, error) {
	if err := ctx.Err(); err != nil {
		return "", err
	}
	req.SessionID = strings.TrimSpace(req.SessionID)
	prompt := strings.TrimSpace(req.Prompt)
	if req.SessionID == "" {
		return "", fmt.Errorf("session id is required")
	}
	if prompt == "" {
		return "", fmt.Errorf("input text is required")
	}

	r.activeMu.RLock()
	control := r.activeRuns[req.SessionID]
	r.activeMu.RUnlock()
	if control == nil {
		return "", ErrNoActiveRun
	}
	if req.ExpectedRunID != "" && req.ExpectedRunID != control.runID {
		return "", fmt.Errorf("%w: expected %s but found %s", ErrActiveRunMismatch, req.ExpectedRunID, control.runID)
	}

	control.mu.Lock()
	control.steers = append(control.steers, prompt)
	control.mu.Unlock()
	r.emit(ctx, eventbus.Event{
		Kind:      eventbus.KindUserSteerSubmitted,
		Source:    eventbus.SourceTUI,
		SessionID: req.SessionID,
		RunID:     control.runID,
		Payload: map[string]any{
			"prompt": prompt,
		},
	})
	return control.runID, nil
}

func (r *Runner) drainSteers(control *activeRun) []string {
	if control == nil {
		return nil
	}
	control.mu.Lock()
	defer control.mu.Unlock()
	if len(control.steers) == 0 {
		return nil
	}
	steers := append([]string(nil), control.steers...)
	control.steers = nil
	return steers
}

func (r *Runner) markMemoryObserverStarted(control *activeRun) bool {
	if control == nil {
		return false
	}
	control.mu.Lock()
	defer control.mu.Unlock()
	if control.observerStarted {
		return false
	}
	control.observerStarted = true
	return true
}

func (r *Runner) appendSteers(ctx context.Context, req RunRequest, messages []provider.Message, inputItems []provider.Item, steers []string) ([]provider.Message, []provider.Item) {
	for _, steer := range steers {
		steer = strings.TrimSpace(steer)
		if steer == "" {
			continue
		}
		r.emit(ctx, eventbus.Event{
			Kind:      eventbus.KindUserSteerApplied,
			Source:    eventbus.SourceAgent,
			SessionID: req.SessionID,
			RunID:     req.RunID,
			Payload: map[string]any{
				"prompt": steer,
			},
		})
		r.emit(ctx, eventbus.Event{
			Kind:      eventbus.KindUserPromptSubmitted,
			Source:    eventbus.SourceTUI,
			SessionID: req.SessionID,
			RunID:     req.RunID,
			Payload: map[string]any{
				"prompt": steer,
				"steer":  true,
			},
		})
		_ = r.persistUserTextItem(ctx, req, "steer:"+uuid.NewString(), steer)
		message := provider.Message{Role: provider.RoleUser, Content: steer}
		messages = append(messages, message)
		inputItems = append(inputItems, providerItemFromProviderMessage(message))
	}
	return messages, inputItems
}

func (r *Runner) persistUserTextItem(ctx context.Context, req RunRequest, id string, text string) error {
	if r.items == nil {
		return nil
	}
	_, err := r.items.Append(ctx, session.Item{
		ID:        req.RunID + ":" + id,
		SessionID: req.SessionID,
		TurnID:    req.RunID,
		Kind:      session.ItemMessage,
		Role:      "user",
		Text:      text,
	})
	return err
}
