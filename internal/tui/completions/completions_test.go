package completions

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/pandelisz/gode/internal/godex/mcp"
	"github.com/pandelisz/gode/internal/godex/skills"
)

func TestFileCompletionCompletesWorkspaceRelativePaths(t *testing.T) {
	root := t.TempDir()
	if err := os.MkdirAll(filepath.Join(root, "internal"), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root, "internal", "main.go"), []byte("package main\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	items := Files(root, "@main", 10)
	if len(items) != 1 || items[0].Path != "internal/main.go" {
		t.Fatalf("items = %#v", items)
	}
}

func TestSkillCompletionCompletesDollarNames(t *testing.T) {
	items := Skills([]skills.Skill{{Name: "some-skill", Description: "does work"}}, "$some", 10)
	if len(items) != 1 || items[0].Name != "some-skill" {
		t.Fatalf("items = %#v", items)
	}
}

func TestResourceCompletionCompletesServerQualifiedQuery(t *testing.T) {
	items := Resources([]mcp.Resource{{Server: "docs", URI: "file://README.md", Name: "README"}}, "@docs:read", 10)
	if len(items) != 1 || items[0].Server != "docs" || items[0].URI != "file://README.md" {
		t.Fatalf("items = %#v", items)
	}
}
