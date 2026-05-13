package memory

import (
	"path/filepath"
	"strings"
	"testing"
)

func TestWorkspaceScopeNormalizesGitPaths(t *testing.T) {
	root := filepath.Join(t.TempDir(), "repo")
	scope, err := NewScope(filepath.Join(root, ".git"), "", t.TempDir())
	if err != nil {
		t.Fatalf("scope: %v", err)
	}
	if scope.WorkspaceRoot != root {
		t.Fatalf("workspace root = %q, want %q", scope.WorkspaceRoot, root)
	}

	scope, err = NewScope(filepath.Join(root, ".git", "worktrees", "feature"), "", t.TempDir())
	if err != nil {
		t.Fatalf("worktree scope: %v", err)
	}
	if scope.WorkspaceRoot != root {
		t.Fatalf("worktree workspace root = %q, want %q", scope.WorkspaceRoot, root)
	}
}

func TestWorkspaceScopeNormalizesRelativePathAndStableID(t *testing.T) {
	root := t.TempDir()
	dataDir := t.TempDir()
	t.Chdir(root)

	first, err := NewScope(".", "", dataDir)
	if err != nil {
		t.Fatalf("scope: %v", err)
	}
	second, err := NewScope("./.", "", dataDir)
	if err != nil {
		t.Fatalf("second scope: %v", err)
	}
	if first.WorkspaceRoot != root {
		t.Fatalf("workspace root = %q, want %q", first.WorkspaceRoot, root)
	}
	if first.WorkspaceID == "" || first.WorkspaceID != second.WorkspaceID {
		t.Fatalf("workspace ids should be stable, got %q and %q", first.WorkspaceID, second.WorkspaceID)
	}
	if strings.Contains(first.WorkspaceID, root) {
		t.Fatalf("workspace id should not expose path: %q", first.WorkspaceID)
	}
	if first.DatabasePath != filepath.Join(dataDir, "memories.sqlite3") {
		t.Fatalf("database path = %q", first.DatabasePath)
	}
}

func TestWorkspaceScopeRejectsEmptyPath(t *testing.T) {
	if _, err := NewScope("", "", t.TempDir()); err == nil {
		t.Fatal("expected empty workspace error")
	}
}
