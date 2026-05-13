package builtin

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex/tools"
)

func TestFilesystemToolsReadListAndSearchWithinRoot(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "main.go"), []byte("package main\nfunc main() {}\n"), 0o600); err != nil {
		t.Fatalf("write: %v", err)
	}

	reg := tools.NewRegistry()
	RegisterFilesystem(reg, root)

	read, err := reg.Run(context.Background(), tools.Call{Name: "read_file", Input: map[string]any{"path": "main.go"}})
	if err != nil {
		t.Fatalf("read: %v", err)
	}
	if read.Text != "package main\nfunc main() {}\n" {
		t.Fatalf("read text = %q", read.Text)
	}

	list, err := reg.Run(context.Background(), tools.Call{Name: "list_files", Input: map[string]any{"path": "."}})
	if err != nil {
		t.Fatalf("list: %v", err)
	}
	if list.Text != "main.go" {
		t.Fatalf("list text = %q", list.Text)
	}

	search, err := reg.Run(context.Background(), tools.Call{Name: "search_files", Input: map[string]any{"query": "func main"}})
	if err != nil {
		t.Fatalf("search: %v", err)
	}
	if search.Text != "main.go:2:func main() {}" {
		t.Fatalf("search text = %q", search.Text)
	}
}

func TestSearchFilesSkipsBinaryFiles(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "text.txt"), []byte("needle in text\n"), 0o600); err != nil {
		t.Fatalf("write text: %v", err)
	}
	if err := os.WriteFile(filepath.Join(root, "binary.bin"), []byte{'n', 'e', 'e', 'd', 'l', 'e', 0, 1, 2}, 0o600); err != nil {
		t.Fatalf("write binary: %v", err)
	}

	reg := tools.NewRegistry()
	RegisterFilesystem(reg, root)

	search, err := reg.Run(context.Background(), tools.Call{Name: "search_files", Input: map[string]any{"query": "needle"}})
	if err != nil {
		t.Fatalf("search: %v", err)
	}
	if strings.Contains(search.Text, "binary.bin") {
		t.Fatalf("search should skip binary files, got:\n%s", search.Text)
	}
	if !strings.Contains(search.Text, "text.txt:1:needle in text") {
		t.Fatalf("search should include text match, got:\n%s", search.Text)
	}
}

func TestSearchFilesSkipsGitignoredFiles(t *testing.T) {
	if _, err := exec.LookPath("git"); err != nil {
		t.Skip("git is required for gitignore test")
	}
	root := t.TempDir()
	runGit(t, root, "init")
	if err := os.WriteFile(filepath.Join(root, ".gitignore"), []byte("ignored.txt\nignored-dir/\n*.log\n"), 0o600); err != nil {
		t.Fatalf("write gitignore: %v", err)
	}
	if err := os.Mkdir(filepath.Join(root, "ignored-dir"), 0o700); err != nil {
		t.Fatalf("mkdir ignored dir: %v", err)
	}
	files := map[string]string{
		"visible.txt":            "needle visible\n",
		"ignored.txt":            "needle ignored\n",
		"ignored.log":            "needle ignored by glob\n",
		"ignored-dir/hidden.txt": "needle ignored in dir\n",
	}
	for name, content := range files {
		if err := os.WriteFile(filepath.Join(root, name), []byte(content), 0o600); err != nil {
			t.Fatalf("write %s: %v", name, err)
		}
	}

	reg := tools.NewRegistry()
	RegisterFilesystem(reg, root)

	search, err := reg.Run(context.Background(), tools.Call{Name: "search_files", Input: map[string]any{"query": "needle"}})
	if err != nil {
		t.Fatalf("search: %v", err)
	}
	if !strings.Contains(search.Text, "visible.txt:1:needle visible") {
		t.Fatalf("search should include visible file, got:\n%s", search.Text)
	}
	for _, ignored := range []string{"ignored.txt", "ignored.log", "ignored-dir"} {
		if strings.Contains(search.Text, ignored) {
			t.Fatalf("search should skip gitignored path %q, got:\n%s", ignored, search.Text)
		}
	}
}

func TestFilesystemToolsRejectPathEscape(t *testing.T) {
	root := t.TempDir()
	reg := tools.NewRegistry()
	RegisterFilesystem(reg, root)

	_, err := reg.Run(context.Background(), tools.Call{Name: "read_file", Input: map[string]any{"path": "../secret"}})
	if err == nil {
		t.Fatal("expected path escape error")
	}
}

func runGit(t *testing.T, dir string, args ...string) {
	t.Helper()
	cmd := exec.Command("git", append([]string{"-C", dir}, args...)...)
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("git %v: %v\n%s", args, err, out)
	}
}
