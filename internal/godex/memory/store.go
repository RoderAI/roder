package memory

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/google/uuid"
	_ "modernc.org/sqlite"
)

var ErrNotFound = errors.New("memory not found")

type Store struct {
	db *sql.DB
}

func OpenStore(ctx context.Context, path string) (*Store, error) {
	path = strings.TrimSpace(path)
	if path == "" {
		return nil, errors.New("memory database path is required")
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return nil, fmt.Errorf("memory database dir: %w", err)
	}
	db, err := sql.Open("sqlite", path)
	if err != nil {
		return nil, err
	}
	store := &Store{db: db}
	if err := migrate(ctx, db); err != nil {
		_ = db.Close()
		return nil, err
	}
	return store, nil
}

func (s *Store) Close() error {
	if s == nil || s.db == nil {
		return nil
	}
	return s.db.Close()
}

func (s *Store) UpsertWorkspace(ctx context.Context, scope Scope) error {
	if scope.WorkspaceID == "" || scope.WorkspaceRoot == "" {
		return errors.New("workspace scope is required")
	}
	now := time.Now().UTC()
	_, err := s.db.ExecContext(ctx, `
		INSERT INTO memory_workspaces (id, root, created_at, updated_at)
		VALUES (?, ?, ?, ?)
		ON CONFLICT(id) DO UPDATE SET root = excluded.root, updated_at = excluded.updated_at
	`, scope.WorkspaceID, scope.WorkspaceRoot, unixTime(now), unixTime(now))
	return err
}

func (s *Store) Save(ctx context.Context, entry Entry, vector Vector) (Entry, error) {
	entry.Content = normalizeContent(entry.Content)
	if entry.Content == "" {
		return Entry{}, errors.New("memory content is required")
	}
	if entry.WorkspaceID == "" {
		return Entry{}, errors.New("workspace id is required")
	}
	if entry.Source == "" {
		entry.Source = "tool"
	}
	entry.ContentHash = contentHash(entry.Content)
	if existing, err := s.findActiveByHash(ctx, entry.WorkspaceID, entry.ContentHash); err == nil {
		return existing, nil
	} else if !errors.Is(err, ErrNotFound) {
		return Entry{}, err
	}
	if entry.ID == "" {
		entry.ID = "mem_" + strings.ReplaceAll(uuid.NewString(), "-", "")
	}
	now := time.Now().UTC()
	entry.CreatedAt = now
	entry.UpdatedAt = now
	if entry.Metadata == nil {
		entry.Metadata = map[string]string{}
	}
	entry.EmbeddingModel = vector.Model
	if vector.Dimensions == 0 {
		vector.Dimensions = len(vector.Values)
	}
	entry.EmbeddingDims = vector.Dimensions
	metadata, err := json.Marshal(entry.Metadata)
	if err != nil {
		return Entry{}, err
	}
	blob, err := EncodeVector(vector)
	if err != nil {
		return Entry{}, err
	}
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return Entry{}, err
	}
	defer tx.Rollback()
	if entry.WorkspaceRoot != "" {
		_, err = tx.ExecContext(ctx, `
			INSERT INTO memory_workspaces (id, root, created_at, updated_at)
			VALUES (?, ?, ?, ?)
			ON CONFLICT(id) DO UPDATE SET root = excluded.root, updated_at = excluded.updated_at
		`, entry.WorkspaceID, entry.WorkspaceRoot, unixTime(now), unixTime(now))
		if err != nil {
			return Entry{}, err
		}
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO memories (id, workspace_id, content, content_hash, source, metadata_json, created_at, updated_at)
		VALUES (?, ?, ?, ?, ?, ?, ?, ?)
	`, entry.ID, entry.WorkspaceID, entry.Content, entry.ContentHash, entry.Source, string(metadata), unixTime(entry.CreatedAt), unixTime(entry.UpdatedAt)); err != nil {
		return Entry{}, err
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO memory_embeddings (memory_id, model, dimensions, embedding, created_at)
		VALUES (?, ?, ?, ?, ?)
	`, entry.ID, vector.Model, vector.Dimensions, blob, unixTime(now)); err != nil {
		return Entry{}, err
	}
	if err := tx.Commit(); err != nil {
		return Entry{}, err
	}
	return entry, nil
}

func (s *Store) Update(ctx context.Context, id string, content string, vector Vector) (Entry, error) {
	id = strings.TrimSpace(id)
	content = normalizeContent(content)
	if id == "" {
		return Entry{}, errors.New("memory id is required")
	}
	if content == "" {
		return Entry{}, errors.New("memory content is required")
	}
	existing, err := s.readByID(ctx, id)
	if err != nil {
		return Entry{}, err
	}
	now := time.Now().UTC()
	contentHash := contentHash(content)
	if vector.Dimensions == 0 {
		vector.Dimensions = len(vector.Values)
	}
	blob, err := EncodeVector(vector)
	if err != nil {
		return Entry{}, err
	}
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return Entry{}, err
	}
	defer tx.Rollback()
	if _, err := tx.ExecContext(ctx, `
		UPDATE memories SET content = ?, content_hash = ?, updated_at = ?
		WHERE id = ? AND deleted_at IS NULL
	`, content, contentHash, unixTime(now), id); err != nil {
		return Entry{}, err
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO memory_embeddings (memory_id, model, dimensions, embedding, created_at)
		VALUES (?, ?, ?, ?, ?)
		ON CONFLICT(memory_id, model) DO UPDATE SET
			dimensions = excluded.dimensions,
			embedding = excluded.embedding,
			created_at = excluded.created_at
	`, id, vector.Model, vector.Dimensions, blob, unixTime(now)); err != nil {
		return Entry{}, err
	}
	if err := tx.Commit(); err != nil {
		return Entry{}, err
	}
	existing.Content = content
	existing.ContentHash = contentHash
	existing.UpdatedAt = now
	existing.EmbeddingModel = vector.Model
	existing.EmbeddingDims = vector.Dimensions
	return existing, nil
}

func (s *Store) SoftDelete(ctx context.Context, id string) error {
	id = strings.TrimSpace(id)
	if id == "" {
		return errors.New("memory id is required")
	}
	_, err := s.db.ExecContext(ctx, `UPDATE memories SET deleted_at = ?, updated_at = ? WHERE id = ? AND deleted_at IS NULL`, unixTime(time.Now().UTC()), unixTime(time.Now().UTC()), id)
	return err
}

func (s *Store) Read(ctx context.Context, workspaceID string, id string) (Entry, error) {
	row := s.db.QueryRowContext(ctx, selectEntrySQL()+` WHERE m.workspace_id = ? AND m.id = ? AND m.deleted_at IS NULL`, workspaceID, id)
	return scanEntry(row)
}

func (s *Store) ListWorkspace(ctx context.Context, workspaceID string, limit int) ([]Entry, error) {
	if limit <= 0 {
		limit = DefaultRecallLimit
	}
	rows, err := s.db.QueryContext(ctx, selectEntrySQL()+`
		WHERE m.workspace_id = ? AND m.deleted_at IS NULL
		ORDER BY m.updated_at DESC
		LIMIT ?
	`, workspaceID, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var entries []Entry
	for rows.Next() {
		entry, err := scanEntry(rows)
		if err != nil {
			return nil, err
		}
		entries = append(entries, entry)
	}
	if err := rows.Err(); err != nil {
		return nil, err
	}
	return entries, nil
}

func (s *Store) Candidates(ctx context.Context, workspaceID string, model string, limit int) ([]Entry, []Vector, error) {
	if limit <= 0 {
		limit = DefaultRecallLimit
	}
	rows, err := s.db.QueryContext(ctx, selectEntryWithVectorSQL()+`
		WHERE m.workspace_id = ? AND e.model = ? AND m.deleted_at IS NULL
		ORDER BY m.updated_at DESC
		LIMIT ?
	`, workspaceID, model, limit)
	if err != nil {
		return nil, nil, err
	}
	defer rows.Close()
	var entries []Entry
	var vectors []Vector
	for rows.Next() {
		entry, vector, err := scanEntryWithVector(rows)
		if err != nil {
			return nil, nil, err
		}
		entries = append(entries, entry)
		vectors = append(vectors, vector)
	}
	if err := rows.Err(); err != nil {
		return nil, nil, err
	}
	return entries, vectors, nil
}

func (s *Store) findActiveByHash(ctx context.Context, workspaceID string, hash string) (Entry, error) {
	row := s.db.QueryRowContext(ctx, selectEntrySQL()+` WHERE m.workspace_id = ? AND m.content_hash = ? AND m.deleted_at IS NULL`, workspaceID, hash)
	return scanEntry(row)
}

func (s *Store) readByID(ctx context.Context, id string) (Entry, error) {
	row := s.db.QueryRowContext(ctx, selectEntrySQL()+` WHERE m.id = ? AND m.deleted_at IS NULL`, id)
	return scanEntry(row)
}

func selectEntrySQL() string {
	return `SELECT m.id, m.workspace_id, w.root, m.content, m.content_hash, m.source, m.metadata_json, m.created_at, m.updated_at, m.deleted_at
		FROM memories m
		LEFT JOIN memory_workspaces w ON w.id = m.workspace_id`
}

func selectEntryWithVectorSQL() string {
	return `SELECT m.id, m.workspace_id, w.root, m.content, m.content_hash, m.source, m.metadata_json, m.created_at, m.updated_at, m.deleted_at,
			e.model, e.dimensions, e.embedding
		FROM memories m
		INNER JOIN memory_embeddings e ON e.memory_id = m.id
		LEFT JOIN memory_workspaces w ON w.id = m.workspace_id`
}

type scanner interface {
	Scan(dest ...any) error
}

func scanEntry(row scanner) (Entry, error) {
	var entry Entry
	var metadataJSON string
	var createdAt, updatedAt int64
	var deletedAt sql.NullInt64
	err := row.Scan(&entry.ID, &entry.WorkspaceID, &entry.WorkspaceRoot, &entry.Content, &entry.ContentHash, &entry.Source, &metadataJSON, &createdAt, &updatedAt, &deletedAt)
	if errors.Is(err, sql.ErrNoRows) {
		return Entry{}, ErrNotFound
	}
	if err != nil {
		return Entry{}, err
	}
	entry.CreatedAt = timeFromUnix(createdAt)
	entry.UpdatedAt = timeFromUnix(updatedAt)
	if deletedAt.Valid {
		t := timeFromUnix(deletedAt.Int64)
		entry.DeletedAt = &t
	}
	if metadataJSON != "" {
		if err := json.Unmarshal([]byte(metadataJSON), &entry.Metadata); err != nil {
			return Entry{}, err
		}
	}
	if entry.Metadata == nil {
		entry.Metadata = map[string]string{}
	}
	return entry, nil
}

func scanEntryWithVector(rows *sql.Rows) (Entry, Vector, error) {
	var entry Entry
	var metadataJSON string
	var createdAt, updatedAt int64
	var deletedAt sql.NullInt64
	var model string
	var dimensions int
	var blob []byte
	err := rows.Scan(&entry.ID, &entry.WorkspaceID, &entry.WorkspaceRoot, &entry.Content, &entry.ContentHash, &entry.Source, &metadataJSON, &createdAt, &updatedAt, &deletedAt, &model, &dimensions, &blob)
	if err != nil {
		return Entry{}, Vector{}, err
	}
	entry.CreatedAt = timeFromUnix(createdAt)
	entry.UpdatedAt = timeFromUnix(updatedAt)
	if deletedAt.Valid {
		t := timeFromUnix(deletedAt.Int64)
		entry.DeletedAt = &t
	}
	if metadataJSON != "" {
		if err := json.Unmarshal([]byte(metadataJSON), &entry.Metadata); err != nil {
			return Entry{}, Vector{}, err
		}
	}
	if entry.Metadata == nil {
		entry.Metadata = map[string]string{}
	}
	entry.EmbeddingModel = model
	entry.EmbeddingDims = dimensions
	vector, err := DecodeVector(model, dimensions, blob)
	if err != nil {
		return Entry{}, Vector{}, err
	}
	return entry, vector, nil
}

func normalizeContent(content string) string {
	return strings.Join(strings.Fields(strings.TrimSpace(content)), " ")
}

func contentHash(content string) string {
	sum := sha256.Sum256([]byte(content))
	return hex.EncodeToString(sum[:])
}

func unixTime(t time.Time) int64 {
	return t.UnixNano()
}

func timeFromUnix(value int64) time.Time {
	return time.Unix(0, value).UTC()
}
