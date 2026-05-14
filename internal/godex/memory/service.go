package memory

import (
	"context"
	"errors"
	"sort"
	"strings"

	"github.com/pandelisz/gode/internal/godex/eventbus"
)

var ErrDisabled = errors.New("memories are disabled")

const (
	KindMemorySaved   eventbus.Kind = "memory.saved"
	KindMemoryUpdated eventbus.Kind = "memory.updated"
	KindMemoryDeleted eventbus.Kind = "memory.deleted"
	KindMemoryQueried eventbus.Kind = "memory.queried"
)

type Service struct {
	store    *Store
	embedder Embedder
	scope    Scope
	cfg      Config
	bus      *eventbus.Bus
}

type Stats struct {
	Enabled        bool
	WorkspaceID    string
	WorkspaceRoot  string
	DatabasePath   string
	EmbeddingModel string
	RecallLimit    int
	Count          int
}

func NewService(store *Store, embedder Embedder, scope Scope, cfg Config, bus *eventbus.Bus) *Service {
	if strings.TrimSpace(cfg.EmbeddingModel) == "" {
		if embedder != nil && strings.TrimSpace(embedder.Model()) != "" {
			cfg.EmbeddingModel = embedder.Model()
		} else {
			cfg.EmbeddingModel = DefaultEmbeddingModel
		}
	}
	if cfg.RecallLimit <= 0 {
		cfg.RecallLimit = DefaultRecallLimit
	}
	if cfg.RecallLimit > MaxRecallLimit {
		cfg.RecallLimit = MaxRecallLimit
	}
	if strings.TrimSpace(cfg.DatabasePath) == "" {
		cfg.DatabasePath = scope.DatabasePath
	}
	return &Service{store: store, embedder: embedder, scope: scope, cfg: cfg, bus: bus}
}

func (s *Service) Save(ctx context.Context, content string, source string) (Entry, error) {
	if err := s.ready(); err != nil {
		return Entry{}, err
	}
	content = normalizeContent(content)
	if content == "" {
		return Entry{}, errors.New("memory content is required")
	}
	vector, err := s.embedder.Embed(ctx, content)
	if err != nil {
		return Entry{}, err
	}
	if err := s.store.UpsertWorkspace(ctx, s.scope); err != nil {
		return Entry{}, err
	}
	entry, err := s.store.Save(ctx, Entry{
		WorkspaceID:   s.scope.WorkspaceID,
		WorkspaceRoot: s.scope.WorkspaceRoot,
		Content:       content,
		Source:        source,
	}, vector)
	if err != nil {
		return Entry{}, err
	}
	s.emit(ctx, KindMemorySaved, map[string]any{"memory_id": entry.ID, "source": entry.Source, "model": vector.Model})
	return entry, nil
}

func (s *Service) Update(ctx context.Context, id string, content string) (Entry, error) {
	if err := s.ready(); err != nil {
		return Entry{}, err
	}
	content = normalizeContent(content)
	if content == "" {
		return Entry{}, errors.New("memory content is required")
	}
	if _, err := s.store.Read(ctx, s.scope.WorkspaceID, id); err != nil {
		return Entry{}, err
	}
	vector, err := s.embedder.Embed(ctx, content)
	if err != nil {
		return Entry{}, err
	}
	entry, err := s.store.Update(ctx, id, content, vector)
	if err != nil {
		return Entry{}, err
	}
	s.emit(ctx, KindMemoryUpdated, map[string]any{"memory_id": entry.ID, "model": vector.Model})
	return entry, nil
}

func (s *Service) Delete(ctx context.Context, id string) error {
	if err := s.ready(); err != nil {
		return err
	}
	if _, err := s.store.Read(ctx, s.scope.WorkspaceID, id); err != nil {
		return err
	}
	if err := s.store.SoftDelete(ctx, id); err != nil {
		return err
	}
	s.emit(ctx, KindMemoryDeleted, map[string]any{"memory_id": id})
	return nil
}

func (s *Service) Query(ctx context.Context, query string, limit int) ([]Entry, error) {
	if err := s.ready(); err != nil {
		return nil, err
	}
	query = normalizeContent(query)
	if query == "" {
		return nil, errors.New("memory query is required")
	}
	limit = s.queryLimit(limit)
	entries, vectors, err := s.store.Candidates(ctx, s.scope.WorkspaceID, s.embedder.Model(), MaxRecallLimit)
	if err != nil {
		return nil, err
	}
	if len(entries) == 0 {
		s.emit(ctx, KindMemoryQueried, map[string]any{"query": query, "count": 0, "model": s.embedder.Model(), "memory_ids": []string{}})
		return nil, nil
	}
	queryVector, err := s.embedder.Embed(ctx, query)
	if err != nil {
		return nil, err
	}
	for i := range entries {
		score, err := Similarity(queryVector, vectors[i])
		if err != nil {
			return nil, err
		}
		entries[i].Score = score
	}
	sort.SliceStable(entries, func(i, j int) bool {
		return entries[i].Score > entries[j].Score
	})
	if len(entries) > limit {
		entries = entries[:limit]
	}
	ids := make([]string, 0, len(entries))
	for _, entry := range entries {
		ids = append(ids, entry.ID)
	}
	s.emit(ctx, KindMemoryQueried, map[string]any{"query": query, "count": len(entries), "model": queryVector.Model, "memory_ids": ids})
	return entries, nil
}

func (s *Service) Read(ctx context.Context, id string) (Entry, error) {
	if err := s.ready(); err != nil {
		return Entry{}, err
	}
	return s.store.Read(ctx, s.scope.WorkspaceID, id)
}

func (s *Service) List(ctx context.Context, limit int) ([]Entry, error) {
	if err := s.ready(); err != nil {
		return nil, err
	}
	return s.store.ListWorkspace(ctx, s.scope.WorkspaceID, s.queryLimit(limit))
}

func (s *Service) Stats(ctx context.Context) (Stats, error) {
	if s == nil {
		return Stats{}, errors.New("memory service is required")
	}
	stats := Stats{
		Enabled:        s.cfg.Enabled,
		WorkspaceID:    s.scope.WorkspaceID,
		WorkspaceRoot:  s.scope.WorkspaceRoot,
		DatabasePath:   s.cfg.DatabasePath,
		EmbeddingModel: s.cfg.EmbeddingModel,
		RecallLimit:    s.cfg.RecallLimit,
	}
	if s.store == nil || s.store.db == nil {
		return stats, nil
	}
	_ = s.store.db.QueryRowContext(ctx, `SELECT COUNT(*) FROM memories WHERE workspace_id = ? AND deleted_at IS NULL`, s.scope.WorkspaceID).Scan(&stats.Count)
	return stats, nil
}

func (s *Service) Close() error {
	if s == nil || s.store == nil {
		return nil
	}
	return s.store.Close()
}

func (s *Service) ready() error {
	if s == nil {
		return errors.New("memory service is required")
	}
	if !s.cfg.Enabled {
		return ErrDisabled
	}
	if s.store == nil {
		return errors.New("memory store is required")
	}
	if s.embedder == nil {
		return errors.New("memory embedder is required")
	}
	if s.scope.WorkspaceID == "" {
		return errors.New("memory workspace scope is required")
	}
	return nil
}

func (s *Service) queryLimit(limit int) int {
	if limit <= 0 {
		limit = s.cfg.RecallLimit
	}
	if limit <= 0 {
		limit = DefaultRecallLimit
	}
	if limit > MaxRecallLimit {
		return MaxRecallLimit
	}
	return limit
}

func (s *Service) emit(ctx context.Context, kind eventbus.Kind, payload any) {
	if s == nil || s.bus == nil {
		return
	}
	s.bus.Publish(ctx, eventbus.Event{
		Kind:    kind,
		Source:  eventbus.SourceSystem,
		Payload: payload,
	})
}
