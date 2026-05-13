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
	"strings"
	"sync"
	"time"

	"github.com/google/uuid"
)

const itemsFileName = "items.jsonl"

type ItemStore struct {
	mu  sync.Mutex
	dir string
	now func() time.Time
}

func OpenItemStore(dataDir string) (*ItemStore, error) {
	dir := filepath.Join(dataDir, "sessions")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return nil, fmt.Errorf("session dir: %w", err)
	}
	return &ItemStore{
		dir: dir,
		now: func() time.Time {
			return time.Now().UTC()
		},
	}, nil
}

func (s *ItemStore) Append(ctx context.Context, item Item) (Item, error) {
	items, err := s.AppendMany(ctx, []Item{item})
	if err != nil {
		return Item{}, err
	}
	return items[0], nil
}

func (s *ItemStore) AppendMany(ctx context.Context, items []Item) ([]Item, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if len(items) == 0 {
		return nil, nil
	}
	normalized := make([]Item, len(items))
	now := s.now()
	sessionID := strings.TrimSpace(items[0].SessionID)
	if sessionID == "" {
		return nil, errors.New("session id is required")
	}
	for i, item := range items {
		if strings.TrimSpace(item.SessionID) == "" {
			item.SessionID = sessionID
		}
		if item.SessionID != sessionID {
			return nil, errors.New("items must belong to one session")
		}
		if strings.TrimSpace(item.ID) == "" {
			item.ID = uuid.NewString()
		}
		if item.Kind == "" {
			item.Kind = ItemRaw
		}
		if item.CreatedAt.IsZero() {
			item.CreatedAt = now
		}
		normalized[i] = item
	}

	s.mu.Lock()
	defer s.mu.Unlock()
	if err := s.appendManyLocked(normalized); err != nil {
		return nil, err
	}
	return normalized, nil
}

func (s *ItemStore) ListBySession(ctx context.Context, sessionID string) ([]Item, error) {
	return s.list(ctx, sessionID, "")
}

func (s *ItemStore) ListByTurn(ctx context.Context, sessionID string, turnID string) ([]Item, error) {
	return s.list(ctx, sessionID, turnID)
}

func (s *ItemStore) appendManyLocked(items []Item) error {
	dir := filepath.Join(s.dir, items[0].SessionID)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return fmt.Errorf("item dir: %w", err)
	}
	path := filepath.Join(dir, itemsFileName)
	file, err := os.OpenFile(path, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0o600)
	if err != nil {
		return fmt.Errorf("open items journal: %w", err)
	}
	encoder := json.NewEncoder(file)
	for _, item := range items {
		if err := encoder.Encode(item); err != nil {
			_ = file.Close()
			return fmt.Errorf("write item: %w", err)
		}
	}
	if err := file.Sync(); err != nil {
		_ = file.Close()
		return fmt.Errorf("sync items journal: %w", err)
	}
	if err := file.Close(); err != nil {
		return fmt.Errorf("close items journal: %w", err)
	}
	return nil
}

func (s *ItemStore) list(ctx context.Context, sessionID string, turnID string) ([]Item, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if strings.TrimSpace(sessionID) == "" {
		return nil, errors.New("session id is required")
	}
	s.mu.Lock()
	defer s.mu.Unlock()

	path := filepath.Join(s.dir, sessionID, itemsFileName)
	file, err := os.Open(path)
	if errors.Is(err, os.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, fmt.Errorf("open items journal: %w", err)
	}
	defer file.Close()

	reader := bufio.NewReader(file)
	var items []Item
	line := 0
	for {
		raw, err := reader.ReadString('\n')
		if errors.Is(err, io.EOF) && raw == "" {
			break
		}
		if err != nil && !errors.Is(err, io.EOF) {
			return nil, fmt.Errorf("read items journal: %w", err)
		}
		line++
		text := strings.TrimSpace(raw)
		if text != "" {
			var item Item
			if err := json.Unmarshal([]byte(text), &item); err != nil {
				return nil, fmt.Errorf("parse items journal %s:%d: %w", path, line, err)
			}
			if item.SessionID == sessionID && (turnID == "" || item.TurnID == turnID) {
				items = append(items, item)
			}
		}
		if errors.Is(err, io.EOF) {
			break
		}
	}
	return items, nil
}
