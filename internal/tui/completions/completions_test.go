package completions

import (
	"os"
	"os/exec"
	"path/filepath"
	"strings"
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

	items := uncachedFiles(root, "@main", 10)
	if len(items) != 1 || items[0].Path != "internal/main.go" {
		t.Fatalf("items = %#v", items)
	}
}

func TestFileCompletionHonorsNestedGitignore(t *testing.T) {
	if _, err := exec.LookPath("git"); err != nil {
		t.Skip("git is required for gitignore test")
	}
	root := t.TempDir()
	runGit(t, root, "init")
	if err := os.MkdirAll(filepath.Join(root, "src", "generated"), 0o700); err != nil {
		t.Fatal(err)
	}
	writes := map[string]string{
		"src/.gitignore":           "ignored.txt\ngenerated/\n",
		"src/visible.txt":          "visible\n",
		"src/ignored.txt":          "ignored\n",
		"src/generated/result.txt": "ignored generated\n",
	}
	for name, content := range writes {
		if err := os.WriteFile(filepath.Join(root, name), []byte(content), 0o600); err != nil {
			t.Fatalf("write %s: %v", name, err)
		}
	}

	items := uncachedFiles(root, "@", 20)
	paths := fileItemPaths(items)
	if !containsPath(paths, "src/.gitignore") || !containsPath(paths, "src/visible.txt") {
		t.Fatalf("expected visible files in %#v", paths)
	}
	for _, ignored := range []string{"src/ignored.txt", "src/generated/result.txt"} {
		if containsPath(paths, ignored) {
			t.Fatalf("file completions should skip nested gitignored path %q, got %#v", ignored, paths)
		}
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

func uncachedFiles(workspace string, query string, limit int) []FileItem {
	workspace = absOrDefault(workspace, ".")
	query = strings.TrimPrefix(strings.TrimSpace(filepath.ToSlash(query)), "@")
	query = strings.ToLower(query)
	all := listWorkspaceFiles(workspace)
	items := make([]FileItem, 0, min(limit, len(all)))
	for _, item := range all {
		if query != "" && !strings.Contains(strings.ToLower(item.Path), query) {
			continue
		}
		items = append(items, item)
		if len(items) >= limit {
			break
		}
	}
	return items
}

func fileItemPaths(items []FileItem) []string {
	paths := make([]string, 0, len(items))
	for _, item := range items {
		paths = append(paths, item.Path)
	}
	return paths
}

func containsPath(paths []string, want string) bool {
	for _, path := range paths {
		if path == want {
			return true
		}
	}
	return false
}

func runGit(t *testing.T, dir string, args ...string) {
	t.Helper()
	cmd := exec.Command("git", append([]string{"-C", dir}, args...)...)
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("git %v: %v\n%s", args, err, out)
	}
}
