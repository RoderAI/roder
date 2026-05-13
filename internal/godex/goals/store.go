package goals

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/google/uuid"
)

const indexFileName = "index.json"

type Store struct {
	mu    sync.Mutex
	dir   string
	index string
	goals map[string]Goal
	now   func() time.Time
}

type indexFile struct {
	Goals []Goal `json:"goals"`
}

func Open(dataDir string) (*Store, error) {
	dir := filepath.Join(dataDir, "goals")
	store := &Store{
		dir:   dir,
		index: filepath.Join(dir, indexFileName),
		goals: map[string]Goal{},
		now: func() time.Time {
			return time.Now().UTC()
		},
	}
	if err := store.load(); err != nil {
		return nil, err
	}
	return store, nil
}

func (s *Store) Get(ctx context.Context, sessionID string) (*Goal, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	sessionID = strings.TrimSpace(sessionID)
	if sessionID == "" {
		return nil, errors.New("session id is required")
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	goal, ok := s.latestForSessionLocked(sessionID)
	if !ok {
		return nil, nil
	}
	return cloneGoal(goal), nil
}

func (s *Store) Set(ctx context.Context, req SetRequest) (*Goal, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	req.SessionID = strings.TrimSpace(req.SessionID)
	if req.SessionID == "" {
		return nil, errors.New("session id is required")
	}
	objective := strings.TrimSpace(req.Objective)
	if objective != "" {
		var err error
		objective, err = ValidateObjective(objective)
		if err != nil {
			return nil, err
		}
	}
	if err := ValidateBudget(req.TokenBudget); err != nil {
		return nil, err
	}
	status := req.Status
	if status == "" {
		status = StatusActive
	}
	if !ValidStatus(status) {
		return nil, ErrInvalidStatus
	}

	s.mu.Lock()
	defer s.mu.Unlock()
	existing, hasExisting := s.latestForSessionLocked(req.SessionID)
	active, hasActive := s.activeForSessionLocked(req.SessionID)
	now := s.now()

	var goal Goal
	switch {
	case objective != "":
		if hasActive && !req.ReplaceExisting {
			return nil, ErrActiveGoalExists
		}
		goal = Goal{
			SessionID:   req.SessionID,
			GoalID:      uuid.NewString(),
			Objective:   objective,
			Status:      status,
			TokenBudget: cloneBudget(req.TokenBudget),
			CreatedAt:   now,
			UpdatedAt:   now,
		}
	case hasActive:
		goal = active
		goal.Status = status
		if req.TokenBudget != nil {
			goal.TokenBudget = cloneBudget(req.TokenBudget)
		}
		goal.UpdatedAt = now
	case hasExisting:
		goal = existing
		goal.Status = status
		if req.TokenBudget != nil {
			goal.TokenBudget = cloneBudget(req.TokenBudget)
		}
		goal.UpdatedAt = now
	default:
		return nil, ErrNotFound
	}

	previous, hadPrevious := s.goals[goal.GoalID]
	s.goals[goal.GoalID] = goal
	if err := s.saveLocked(); err != nil {
		if hadPrevious {
			s.goals[goal.GoalID] = previous
		} else {
			delete(s.goals, goal.GoalID)
		}
		return nil, err
	}
	return cloneGoal(goal), nil
}

func (s *Store) Clear(ctx context.Context, sessionID string) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	sessionID = strings.TrimSpace(sessionID)
	if sessionID == "" {
		return errors.New("session id is required")
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	goal, ok := s.latestForSessionLocked(sessionID)
	if !ok {
		return nil
	}
	delete(s.goals, goal.GoalID)
	if err := s.saveLocked(); err != nil {
		s.goals[goal.GoalID] = goal
		return err
	}
	return nil
}

func (s *Store) AddUsage(ctx context.Context, sessionID string, tokens int64, elapsed time.Duration) (*Goal, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if tokens < 0 {
		tokens = 0
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	goal, ok := s.activeForSessionLocked(sessionID)
	if !ok {
		return nil, nil
	}
	previous := goal
	goal.TokensUsed += tokens
	goal.TimeUsedSeconds += int64(elapsed.Round(time.Second).Seconds())
	if goal.TokenBudget != nil && goal.TokensUsed >= *goal.TokenBudget {
		goal.Status = StatusBudgetLimited
	}
	goal.UpdatedAt = s.now()
	s.goals[goal.GoalID] = goal
	if err := s.saveLocked(); err != nil {
		s.goals[goal.GoalID] = previous
		return nil, err
	}
	return cloneGoal(goal), nil
}

func (s *Store) latestForSessionLocked(sessionID string) (Goal, bool) {
	var latest Goal
	found := false
	for _, goal := range s.goals {
		if goal.SessionID != sessionID {
			continue
		}
		if !found || goal.UpdatedAt.After(latest.UpdatedAt) {
			latest = goal
			found = true
		}
	}
	return latest, found
}

func (s *Store) activeForSessionLocked(sessionID string) (Goal, bool) {
	var latest Goal
	found := false
	for _, goal := range s.goals {
		if goal.SessionID != sessionID || goal.Status == StatusComplete {
			continue
		}
		if !found || goal.UpdatedAt.After(latest.UpdatedAt) {
			latest = goal
			found = true
		}
	}
	return latest, found
}

func (s *Store) load() error {
	data, err := os.ReadFile(s.index)
	if errors.Is(err, os.ErrNotExist) {
		return nil
	}
	if err != nil {
		return fmt.Errorf("read goal index: %w", err)
	}
	if strings.TrimSpace(string(data)) == "" {
		return nil
	}
	var file indexFile
	if err := json.Unmarshal(data, &file); err != nil {
		return fmt.Errorf("parse goal index: %w", err)
	}
	for _, goal := range file.Goals {
		if goal.GoalID != "" {
			s.goals[goal.GoalID] = goal
		}
	}
	return nil
}

func (s *Store) saveLocked() error {
	if err := os.MkdirAll(s.dir, 0o700); err != nil {
		return fmt.Errorf("goal dir: %w", err)
	}
	goals := make([]Goal, 0, len(s.goals))
	for _, goal := range s.goals {
		goals = append(goals, goal)
	}
	sort.Slice(goals, func(i, j int) bool { return goals[i].UpdatedAt.After(goals[j].UpdatedAt) })
	data, err := json.MarshalIndent(indexFile{Goals: goals}, "", "  ")
	if err != nil {
		return fmt.Errorf("encode goal index: %w", err)
	}
	tmp, err := os.CreateTemp(s.dir, ".index-*.tmp")
	if err != nil {
		return fmt.Errorf("create goal index temp: %w", err)
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName)
	if _, err := tmp.Write(data); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("write goal index temp: %w", err)
	}
	if _, err := tmp.Write([]byte("\n")); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("write goal index newline: %w", err)
	}
	if err := tmp.Sync(); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("sync goal index temp: %w", err)
	}
	if err := tmp.Close(); err != nil {
		return fmt.Errorf("close goal index temp: %w", err)
	}
	if err := os.Rename(tmpName, s.index); err != nil {
		return fmt.Errorf("rename goal index: %w", err)
	}
	return nil
}

func cloneGoal(goal Goal) *Goal {
	goal.TokenBudget = cloneBudget(goal.TokenBudget)
	return &goal
}

func cloneBudget(budget *int64) *int64 {
	if budget == nil {
		return nil
	}
	value := *budget
	return &value
}
