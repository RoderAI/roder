package journal

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"sort"
	"strings"
	"sync"

	"github.com/pandelisz/gode/internal/godex/eventbus"
)

type ReplayFilter struct {
	SessionID string
	RunID     string
	Kinds     []eventbus.Kind
}

type Store struct {
	path string
	file *os.File
	mu   sync.Mutex
}

func Open(path string) (*Store, error) {
	file, err := os.OpenFile(path, os.O_CREATE|os.O_APPEND|os.O_RDWR, 0o600)
	if err != nil {
		return nil, err
	}
	return &Store{path: path, file: file}, nil
}

func (s *Store) Path() string {
	if s == nil {
		return ""
	}
	return s.path
}

func (s *Store) Append(ctx context.Context, event eventbus.Event) error {
	select {
	case <-ctx.Done():
		return ctx.Err()
	default:
	}

	data, err := json.Marshal(event)
	if err != nil {
		return err
	}

	s.mu.Lock()
	defer s.mu.Unlock()
	if s.file == nil {
		return errors.New("journal closed")
	}
	if _, err := s.file.Write(append(data, '\n')); err != nil {
		return err
	}
	return nil
}

func (s *Store) Replay(ctx context.Context, filter ReplayFilter) ([]eventbus.Event, error) {
	file, err := os.Open(s.path)
	if err != nil {
		return nil, err
	}
	defer file.Close()

	var events []eventbus.Event
	reader := bufio.NewReader(file)
	line := 0
	for {
		raw, err := reader.ReadString('\n')
		if errors.Is(err, io.EOF) && raw == "" {
			break
		}
		if err != nil && !errors.Is(err, io.EOF) {
			return nil, fmt.Errorf("read journal %s:%d: %w", s.path, line+1, err)
		}
		line++
		select {
		case <-ctx.Done():
			return nil, ctx.Err()
		default:
		}
		text := strings.TrimSpace(raw)
		if text == "" {
			if errors.Is(err, io.EOF) {
				break
			}
			continue
		}
		var ev eventbus.Event
		if err := json.Unmarshal([]byte(text), &ev); err != nil {
			return nil, fmt.Errorf("parse journal %s:%d: %w", s.path, line, err)
		}
		if match(filter, ev) {
			events = append(events, ev)
		}
		if errors.Is(err, io.EOF) {
			break
		}
	}
	return events, nil
}

func (s *Store) ListSessions(ctx context.Context) ([]string, error) {
	events, err := s.Replay(ctx, ReplayFilter{})
	if err != nil {
		return nil, err
	}
	seen := make(map[string]struct{})
	for _, ev := range events {
		if ev.SessionID != "" {
			seen[ev.SessionID] = struct{}{}
		}
	}
	sessions := make([]string, 0, len(seen))
	for session := range seen {
		sessions = append(sessions, session)
	}
	sort.Strings(sessions)
	return sessions, nil
}

func (s *Store) Flush() error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.file == nil {
		return nil
	}
	return s.file.Sync()
}

func (s *Store) Close() error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.file == nil {
		return nil
	}
	err := s.file.Close()
	s.file = nil
	return err
}

func match(filter ReplayFilter, ev eventbus.Event) bool {
	if filter.SessionID != "" && ev.SessionID != filter.SessionID {
		return false
	}
	if filter.RunID != "" && ev.RunID != filter.RunID {
		return false
	}
	if len(filter.Kinds) > 0 {
		for _, kind := range filter.Kinds {
			if ev.Kind == kind {
				return true
			}
		}
		return false
	}
	return true
}
