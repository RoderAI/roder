package session

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

var ErrNotFound = errors.New("session not found")

type Store struct {
	mu       sync.Mutex
	dir      string
	index    string
	sessions map[string]Session
	now      func() time.Time
}

type indexFile struct {
	Sessions []Session `json:"sessions"`
}

func Open(dataDir string) (*Store, error) {
	dir := filepath.Join(dataDir, "sessions")
	store := &Store{
		dir:      dir,
		index:    filepath.Join(dir, indexFileName),
		sessions: map[string]Session{},
		now: func() time.Time {
			return time.Now().UTC()
		},
	}
	if err := store.load(); err != nil {
		return nil, err
	}
	return store, nil
}

func (s *Store) Create(ctx context.Context, title string, parentSessionID string) (Session, error) {
	if err := ctx.Err(); err != nil {
		return Session{}, err
	}
	title = strings.TrimSpace(title)
	now := s.now()
	session := Session{
		ID:              uuid.NewString(),
		Title:           title,
		ParentSessionID: parentSessionID,
		CreatedAt:       now,
		UpdatedAt:       now,
	}

	s.mu.Lock()
	defer s.mu.Unlock()
	s.sessions[session.ID] = session
	if err := s.saveLocked(); err != nil {
		delete(s.sessions, session.ID)
		return Session{}, err
	}
	return session, nil
}

func (s *Store) Ensure(ctx context.Context, session Session) (Session, error) {
	if err := ctx.Err(); err != nil {
		return Session{}, err
	}
	if strings.TrimSpace(session.ID) == "" {
		return Session{}, errors.New("session id is required")
	}
	now := s.now()
	session.Title = strings.TrimSpace(session.Title)

	s.mu.Lock()
	defer s.mu.Unlock()
	if existing, ok := s.sessions[session.ID]; ok {
		if session.Title != "" && existing.Title == "" {
			existing.Title = session.Title
		}
		existing.UpdatedAt = now
		s.sessions[session.ID] = existing
		if err := s.saveLocked(); err != nil {
			return Session{}, err
		}
		return existing, nil
	}
	if session.CreatedAt.IsZero() {
		session.CreatedAt = now
	}
	if session.UpdatedAt.IsZero() {
		session.UpdatedAt = now
	}
	s.sessions[session.ID] = session
	if err := s.saveLocked(); err != nil {
		delete(s.sessions, session.ID)
		return Session{}, err
	}
	return session, nil
}

func (s *Store) UpdateMessageCount(ctx context.Context, id string, count int) (Session, error) {
	if err := ctx.Err(); err != nil {
		return Session{}, err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	session, ok := s.sessions[id]
	if !ok {
		return Session{}, ErrNotFound
	}
	previous := session
	session.MessageCount = count
	session.UpdatedAt = s.now()
	s.sessions[id] = session
	if err := s.saveLocked(); err != nil {
		s.sessions[id] = previous
		return Session{}, err
	}
	return session, nil
}

func (s *Store) Get(ctx context.Context, id string) (Session, bool, error) {
	if err := ctx.Err(); err != nil {
		return Session{}, false, err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	session, ok := s.sessions[id]
	return session, ok, nil
}

func (s *Store) List(ctx context.Context) ([]Session, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	sessions := make([]Session, 0, len(s.sessions))
	for _, session := range s.sessions {
		sessions = append(sessions, session)
	}
	sortSessions(sessions)
	return sessions, nil
}

func (s *Store) Last(ctx context.Context) (Session, bool, error) {
	sessions, err := s.List(ctx)
	if err != nil {
		return Session{}, false, err
	}
	if len(sessions) == 0 {
		return Session{}, false, nil
	}
	return sessions[0], true, nil
}

func (s *Store) LastSession(ctx context.Context) (Session, bool, error) {
	return s.Last(ctx)
}

func (s *Store) Rename(ctx context.Context, id string, title string) (Session, error) {
	if err := ctx.Err(); err != nil {
		return Session{}, err
	}
	title = strings.TrimSpace(title)
	s.mu.Lock()
	defer s.mu.Unlock()
	session, ok := s.sessions[id]
	if !ok {
		return Session{}, ErrNotFound
	}
	previous := session
	session.Title = title
	session.UpdatedAt = s.now()
	s.sessions[id] = session
	if err := s.saveLocked(); err != nil {
		s.sessions[id] = previous
		return Session{}, err
	}
	return session, nil
}

func (s *Store) Delete(ctx context.Context, id string) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	previous, ok := s.sessions[id]
	if !ok {
		return ErrNotFound
	}
	delete(s.sessions, id)
	if err := s.saveLocked(); err != nil {
		s.sessions[id] = previous
		return err
	}
	return nil
}

func (s *Store) load() error {
	data, err := os.ReadFile(s.index)
	if errors.Is(err, os.ErrNotExist) {
		return nil
	}
	if err != nil {
		return fmt.Errorf("read session index: %w", err)
	}
	if strings.TrimSpace(string(data)) == "" {
		return nil
	}
	var file indexFile
	if err := json.Unmarshal(data, &file); err != nil {
		return fmt.Errorf("parse session index: %w", err)
	}
	for _, session := range file.Sessions {
		if session.ID != "" {
			s.sessions[session.ID] = session
		}
	}
	return nil
}

func (s *Store) saveLocked() error {
	if err := os.MkdirAll(s.dir, 0o700); err != nil {
		return fmt.Errorf("session dir: %w", err)
	}
	sessions := make([]Session, 0, len(s.sessions))
	for _, session := range s.sessions {
		sessions = append(sessions, session)
	}
	sortSessions(sessions)
	data, err := json.MarshalIndent(indexFile{Sessions: sessions}, "", "  ")
	if err != nil {
		return fmt.Errorf("encode session index: %w", err)
	}
	tmp, err := os.CreateTemp(s.dir, ".index-*.tmp")
	if err != nil {
		return fmt.Errorf("create session index temp: %w", err)
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName)
	if _, err := tmp.Write(data); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("write session index temp: %w", err)
	}
	if _, err := tmp.Write([]byte("\n")); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("write session index newline: %w", err)
	}
	if err := tmp.Sync(); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("sync session index temp: %w", err)
	}
	if err := tmp.Close(); err != nil {
		return fmt.Errorf("close session index temp: %w", err)
	}
	if err := os.Rename(tmpName, s.index); err != nil {
		return fmt.Errorf("replace session index: %w", err)
	}
	if err := syncDir(s.dir); err != nil {
		return fmt.Errorf("sync session dir: %w", err)
	}
	return nil
}

func syncDir(dir string) error {
	handle, err := os.Open(dir)
	if err != nil {
		return err
	}
	defer handle.Close()
	return handle.Sync()
}

func sortSessions(sessions []Session) {
	sort.SliceStable(sessions, func(i, j int) bool {
		if sessions[i].UpdatedAt.Equal(sessions[j].UpdatedAt) {
			return sessions[i].ID < sessions[j].ID
		}
		return sessions[i].UpdatedAt.After(sessions[j].UpdatedAt)
	})
}
