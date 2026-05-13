package memory

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"testing"
)

func TestStoreOpenCreatesDatabaseAndMigration(t *testing.T) {
	ctx := context.Background()
	path := filepath.Join(t.TempDir(), "nested", "memories.sqlite3")
	store, err := OpenStore(ctx, path)
	if err != nil {
		t.Fatalf("open store: %v", err)
	}
	defer store.Close()

	info, err := os.Stat(filepath.Dir(path))
	if err != nil {
		t.Fatalf("stat db dir: %v", err)
	}
	if info.Mode().Perm() != 0o700 {
		t.Fatalf("db dir mode = %o", info.Mode().Perm())
	}
	var version int
	if err := store.db.QueryRowContext(ctx, `PRAGMA user_version`).Scan(&version); err != nil {
		t.Fatalf("user_version: %v", err)
	}
	if version != 1 {
		t.Fatalf("user_version = %d", version)
	}
}

func TestStoreUpsertWorkspaceReusesID(t *testing.T) {
	ctx := context.Background()
	store := openTestStore(t)
	defer store.Close()
	scope := testScope(t, "repo")

	if err := store.UpsertWorkspace(ctx, scope); err != nil {
		t.Fatalf("upsert workspace: %v", err)
	}
	if err := store.UpsertWorkspace(ctx, scope); err != nil {
		t.Fatalf("second upsert workspace: %v", err)
	}
	var count int
	if err := store.db.QueryRowContext(ctx, `SELECT COUNT(*) FROM memory_workspaces WHERE id = ?`, scope.WorkspaceID).Scan(&count); err != nil {
		t.Fatalf("count: %v", err)
	}
	if count != 1 {
		t.Fatalf("workspace rows = %d", count)
	}
}

func TestStoreSaveUpdateSoftDeleteReadAndCandidates(t *testing.T) {
	ctx := context.Background()
	store := openTestStore(t)
	defer store.Close()
	scope := testScope(t, "repo")
	if err := store.UpsertWorkspace(ctx, scope); err != nil {
		t.Fatalf("upsert workspace: %v", err)
	}

	saved, err := store.Save(ctx, Entry{
		WorkspaceID:   scope.WorkspaceID,
		WorkspaceRoot: scope.WorkspaceRoot,
		Content:       "prefer event bus",
		Source:        "tool",
		Metadata:      map[string]string{"kind": "preference"},
	}, Vector{Model: "embed", Dimensions: 2, Values: []float32{1, 0}})
	if err != nil {
		t.Fatalf("save: %v", err)
	}
	if saved.ID == "" || saved.ContentHash == "" || saved.CreatedAt.IsZero() || saved.UpdatedAt.IsZero() {
		t.Fatalf("saved entry missing fields: %#v", saved)
	}

	read, err := store.Read(ctx, scope.WorkspaceID, saved.ID)
	if err != nil {
		t.Fatalf("read: %v", err)
	}
	if read.Content != "prefer event bus" || read.Metadata["kind"] != "preference" {
		t.Fatalf("read = %#v", read)
	}
	listed, err := store.ListWorkspace(ctx, scope.WorkspaceID, 10)
	if err != nil {
		t.Fatalf("list workspace: %v", err)
	}
	if len(listed) != 1 || listed[0].ID != saved.ID {
		t.Fatalf("listed = %#v", listed)
	}

	updated, err := store.Update(ctx, saved.ID, "prefer sqlite memories", Vector{Model: "embed", Dimensions: 2, Values: []float32{0, 1}})
	if err != nil {
		t.Fatalf("update: %v", err)
	}
	if updated.Content != "prefer sqlite memories" || updated.ContentHash == saved.ContentHash {
		t.Fatalf("updated = %#v", updated)
	}

	entries, vectors, err := store.Candidates(ctx, scope.WorkspaceID, "embed", 10)
	if err != nil {
		t.Fatalf("candidates: %v", err)
	}
	if len(entries) != 1 || len(vectors) != 1 || entries[0].ID != saved.ID || vectors[0].Values[1] != 1 {
		t.Fatalf("candidates = %#v %#v", entries, vectors)
	}

	if err := store.SoftDelete(ctx, saved.ID); err != nil {
		t.Fatalf("soft delete: %v", err)
	}
	if _, err := store.Read(ctx, scope.WorkspaceID, saved.ID); !errors.Is(err, ErrNotFound) {
		t.Fatalf("read deleted err = %v", err)
	}
	listed, err = store.ListWorkspace(ctx, scope.WorkspaceID, 10)
	if err != nil {
		t.Fatalf("list workspace after delete: %v", err)
	}
	if len(listed) != 0 {
		t.Fatalf("deleted memory leaked into list = %#v", listed)
	}
	entries, vectors, err = store.Candidates(ctx, scope.WorkspaceID, "embed", 10)
	if err != nil {
		t.Fatalf("candidates after delete: %v", err)
	}
	if len(entries) != 0 || len(vectors) != 0 {
		t.Fatalf("deleted memory leaked into candidates = %#v %#v", entries, vectors)
	}
}

func TestStoreDuplicateActiveContentReturnsExistingMemoryID(t *testing.T) {
	ctx := context.Background()
	store := openTestStore(t)
	defer store.Close()
	scope := testScope(t, "repo")
	if err := store.UpsertWorkspace(ctx, scope); err != nil {
		t.Fatalf("upsert workspace: %v", err)
	}
	entry := Entry{WorkspaceID: scope.WorkspaceID, WorkspaceRoot: scope.WorkspaceRoot, Content: "same content", Source: "tool"}
	first, err := store.Save(ctx, entry, Vector{Model: "embed", Dimensions: 1, Values: []float32{1}})
	if err != nil {
		t.Fatalf("save first: %v", err)
	}
	second, err := store.Save(ctx, entry, Vector{Model: "embed", Dimensions: 1, Values: []float32{1}})
	if err != nil {
		t.Fatalf("save second: %v", err)
	}
	if second.ID != first.ID {
		t.Fatalf("duplicate id = %q, want %q", second.ID, first.ID)
	}
}

func TestStoreCandidatesDoNotCrossWorkspaceBoundaries(t *testing.T) {
	ctx := context.Background()
	store := openTestStore(t)
	defer store.Close()
	first := testScope(t, "repo-a")
	second := testScope(t, "repo-b")
	for _, scope := range []Scope{first, second} {
		if err := store.UpsertWorkspace(ctx, scope); err != nil {
			t.Fatalf("upsert workspace: %v", err)
		}
	}
	if _, err := store.Save(ctx, Entry{WorkspaceID: first.WorkspaceID, WorkspaceRoot: first.WorkspaceRoot, Content: "first", Source: "tool"}, Vector{Model: "embed", Dimensions: 1, Values: []float32{1}}); err != nil {
		t.Fatalf("save first: %v", err)
	}
	if _, err := store.Save(ctx, Entry{WorkspaceID: second.WorkspaceID, WorkspaceRoot: second.WorkspaceRoot, Content: "second", Source: "tool"}, Vector{Model: "embed", Dimensions: 1, Values: []float32{1}}); err != nil {
		t.Fatalf("save second: %v", err)
	}

	entries, _, err := store.Candidates(ctx, first.WorkspaceID, "embed", 10)
	if err != nil {
		t.Fatalf("candidates: %v", err)
	}
	if len(entries) != 1 || entries[0].Content != "first" {
		t.Fatalf("workspace candidates = %#v", entries)
	}
}

func openTestStore(t *testing.T) *Store {
	t.Helper()
	store, err := OpenStore(context.Background(), filepath.Join(t.TempDir(), "memories.sqlite3"))
	if err != nil {
		t.Fatalf("open store: %v", err)
	}
	return store
}

func testScope(t *testing.T, name string) Scope {
	t.Helper()
	root := filepath.Join(t.TempDir(), name)
	scope, err := NewScope(root, filepath.Join(t.TempDir(), "memories.sqlite3"), t.TempDir())
	if err != nil {
		t.Fatalf("scope: %v", err)
	}
	return scope
}
