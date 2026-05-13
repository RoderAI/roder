package session

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/google/uuid"
)

const turnsFileName = "turns.jsonl"

type TurnStore struct {
	mu  sync.Mutex
	dir string
	now func() time.Time
}

func OpenTurnStore(dataDir string) (*TurnStore, error) {
	dir := filepath.Join(dataDir, "sessions")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return nil, fmt.Errorf("session dir: %w", err)
	}
	return &TurnStore{
		dir: dir,
		now: func() time.Time {
			return time.Now().UTC()
		},
	}, nil
}

func (s *TurnStore) Append(ctx context.Context, turn Turn) (Turn, error) {
	if err := ctx.Err(); err != nil {
		return Turn{}, err
	}
	if strings.TrimSpace(turn.SessionID) == "" {
		return Turn{}, errors.New("session id is required")
	}
	now := s.now()
	if strings.TrimSpace(turn.ID) == "" {
		turn.ID = uuid.NewString()
	}
	if strings.TrimSpace(turn.Status) == "" {
		turn.Status = TurnStatusRunning
	}
	if turn.StartedAt.IsZero() {
		turn.StartedAt = now
	}
	if turn.UpdatedAt.IsZero() {
		turn.UpdatedAt = turn.StartedAt
	}
	return turn, s.append(ctx, turn)
}

func (s *TurnStore) Complete(ctx context.Context, sessionID string, turnID string, responseID string) (Turn, error) {
	return s.update(ctx, sessionID, turnID, func(turn Turn, now time.Time) Turn {
		turn.Status = TurnStatusCompleted
		turn.ResponseID = responseID
		turn.Error = ""
		turn.CompletedAt = now
		turn.UpdatedAt = now
		return turn
	})
}

func (s *TurnStore) Fail(ctx context.Context, sessionID string, turnID string, errorText string) (Turn, error) {
	return s.update(ctx, sessionID, turnID, func(turn Turn, now time.Time) Turn {
		turn.Status = TurnStatusFailed
		turn.Error = errorText
		turn.CompletedAt = now
		turn.UpdatedAt = now
		return turn
	})
}

func (s *TurnStore) ListBySession(ctx context.Context, sessionID string) ([]Turn, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if strings.TrimSpace(sessionID) == "" {
		return nil, errors.New("session id is required")
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.listBySessionLocked(sessionID)
}

func (s *TurnStore) update(ctx context.Context, sessionID string, turnID string, mutate func(Turn, time.Time) Turn) (Turn, error) {
	if err := ctx.Err(); err != nil {
		return Turn{}, err
	}
	if strings.TrimSpace(sessionID) == "" {
		return Turn{}, errors.New("session id is required")
	}
	if strings.TrimSpace(turnID) == "" {
		return Turn{}, errors.New("turn id is required")
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	turns, err := s.listBySessionLocked(sessionID)
	if err != nil {
		return Turn{}, err
	}
	for _, turn := range turns {
		if turn.ID == turnID {
			updated := mutate(turn, s.now())
			return updated, s.appendLocked(updated)
		}
	}
	return Turn{}, ErrNotFound
}

func (s *TurnStore) append(ctx context.Context, turn Turn) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.appendLocked(turn)
}

func (s *TurnStore) appendLocked(turn Turn) error {
	dir := filepath.Join(s.dir, turn.SessionID)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return fmt.Errorf("turn dir: %w", err)
	}
	path := filepath.Join(dir, turnsFileName)
	file, err := os.OpenFile(path, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0o600)
	if err != nil {
		return fmt.Errorf("open turns journal: %w", err)
	}
	encoder := json.NewEncoder(file)
	if err := encoder.Encode(turn); err != nil {
		_ = file.Close()
		return fmt.Errorf("write turn: %w", err)
	}
	if err := file.Sync(); err != nil {
		_ = file.Close()
		return fmt.Errorf("sync turns journal: %w", err)
	}
	if err := file.Close(); err != nil {
		return fmt.Errorf("close turns journal: %w", err)
	}
	return nil
}

func (s *TurnStore) listBySessionLocked(sessionID string) ([]Turn, error) {
	path := filepath.Join(s.dir, sessionID, turnsFileName)
	file, err := os.Open(path)
	if errors.Is(err, os.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, fmt.Errorf("open turns journal: %w", err)
	}
	defer file.Close()

	latest := map[string]Turn{}
	reader := bufio.NewReader(file)
	line := 0
	for {
		raw, err := reader.ReadString('\n')
		if errors.Is(err, io.EOF) && raw == "" {
			break
		}
		if err != nil && !errors.Is(err, io.EOF) {
			return nil, fmt.Errorf("read turns journal: %w", err)
		}
		line++
		text := strings.TrimSpace(raw)
		if text == "" {
			if errors.Is(err, io.EOF) {
				break
			}
			continue
		}
		var turn Turn
		if err := json.Unmarshal([]byte(text), &turn); err != nil {
			return nil, fmt.Errorf("parse turns journal %s:%d: %w", path, line, err)
		}
		if turn.SessionID != sessionID || turn.ID == "" {
			if errors.Is(err, io.EOF) {
				break
			}
			continue
		}
		latest[turn.ID] = turn
		if errors.Is(err, io.EOF) {
			break
		}
	}

	turns := make([]Turn, 0, len(latest))
	for _, turn := range latest {
		turns = append(turns, turn)
	}
	sort.SliceStable(turns, func(i, j int) bool {
		left := turnUpdatedAt(turns[i])
		right := turnUpdatedAt(turns[j])
		if left.Equal(right) {
			return turns[i].ID < turns[j].ID
		}
		return left.After(right)
	})
	return turns, nil
}

func turnUpdatedAt(turn Turn) time.Time {
	if !turn.UpdatedAt.IsZero() {
		return turn.UpdatedAt
	}
	if !turn.CompletedAt.IsZero() {
		return turn.CompletedAt
	}
	return turn.StartedAt
}
