package builtin

import (
	"context"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex/tools"
)

func TestEditingToolsWriteEditAndMultiEdit(t *testing.T) {
	root := t.TempDir()
	reg := tools.NewRegistry()
	RegisterEditing(reg, root)

	write, err := reg.Run(context.Background(), tools.Call{Name: "write_file", Input: map[string]any{"path": "src/main.go", "content": "alpha\nbeta\ngamma\n"}})
	if err != nil {
		t.Fatalf("write: %v", err)
	}
	if !strings.Contains(write.Text, "src/main.go") {
		t.Fatalf("write text = %q", write.Text)
	}

	edit, err := reg.Run(context.Background(), tools.Call{Name: "edit", Input: map[string]any{"path": "src/main.go", "old_string": "beta", "new_string": "BETA"}})
	if err != nil {
		t.Fatalf("edit: %v", err)
	}
	if !strings.Contains(edit.Text, "edited") {
		t.Fatalf("edit text = %q", edit.Text)
	}

	multi, err := reg.Run(context.Background(), tools.Call{Name: "multi_edit", Input: map[string]any{
		"path": "src/main.go",
		"edits": []any{
			map[string]any{"old_string": "alpha", "new_string": "ALPHA"},
			map[string]any{"old_string": "gamma", "new_string": "GAMMA"},
		},
	}})
	if err != nil {
		t.Fatalf("multi_edit: %v", err)
	}
	if !strings.Contains(multi.Text, "2 replacements") {
		t.Fatalf("multi text = %q", multi.Text)
	}
	data, err := os.ReadFile(filepath.Join(root, "src/main.go"))
	if err != nil {
		t.Fatalf("read edited file: %v", err)
	}
	if string(data) != "ALPHA\nBETA\nGAMMA\n" {
		t.Fatalf("edited file = %q", string(data))
	}

	failed, err := reg.Run(context.Background(), tools.Call{Name: "edit", Input: map[string]any{"path": "src/main.go", "old_string": "missing", "new_string": "x"}})
	if err != nil {
		t.Fatalf("failed edit should be a tool result: %v", err)
	}
	if failed.Error == "" || !strings.Contains(failed.Text, "does not match") {
		t.Fatalf("failed edit = %#v", failed)
	}
}

func TestSearchToolsGrepAndGlob(t *testing.T) {
	root := t.TempDir()
	if err := os.MkdirAll(filepath.Join(root, "src"), 0o700); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(root, "src/main.go"), []byte("package main\nfunc main() {}\n"), 0o600); err != nil {
		t.Fatalf("write go: %v", err)
	}
	if err := os.WriteFile(filepath.Join(root, "README.md"), []byte("needle here\n"), 0o600); err != nil {
		t.Fatalf("write readme: %v", err)
	}
	reg := tools.NewRegistry()
	RegisterSearch(reg, root)

	grep, err := reg.Run(context.Background(), tools.Call{Name: "grep", Input: map[string]any{"query": "needle"}})
	if err != nil {
		t.Fatalf("grep: %v", err)
	}
	if !strings.Contains(grep.Text, "README.md:1:needle here") {
		t.Fatalf("grep text = %q", grep.Text)
	}
	glob, err := reg.Run(context.Background(), tools.Call{Name: "glob", Input: map[string]any{"pattern": "src/*.go"}})
	if err != nil {
		t.Fatalf("glob: %v", err)
	}
	if strings.TrimSpace(glob.Text) != "src/main.go" {
		t.Fatalf("glob text = %q", glob.Text)
	}
}

func TestDownloadTool(t *testing.T) {
	root := t.TempDir()
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte("downloaded"))
	}))
	defer server.Close()

	reg := tools.NewRegistry()
	RegisterDownload(reg, root)
	result, err := reg.Run(context.Background(), tools.Call{Name: "download", Input: map[string]any{"url": server.URL, "path": "downloads/file.txt"}})
	if err != nil {
		t.Fatalf("download: %v", err)
	}
	if !strings.Contains(result.Text, "downloaded 10 bytes") {
		t.Fatalf("download text = %q", result.Text)
	}
	data, err := os.ReadFile(filepath.Join(root, "downloads/file.txt"))
	if err != nil {
		t.Fatalf("read download: %v", err)
	}
	if string(data) != "downloaded" {
		t.Fatalf("downloaded data = %q", string(data))
	}
}

func TestGitTools(t *testing.T) {
	if _, err := exec.LookPath("git"); err != nil {
		t.Skip("git is required")
	}
	root := t.TempDir()
	runGit(t, root, "init")
	if err := os.WriteFile(filepath.Join(root, "new.txt"), []byte("new\n"), 0o600); err != nil {
		t.Fatalf("write: %v", err)
	}
	reg := tools.NewRegistry()
	RegisterGit(reg, root)

	status, err := reg.Run(context.Background(), tools.Call{Name: "git_status"})
	if err != nil {
		t.Fatalf("git_status: %v", err)
	}
	if !strings.Contains(status.Text, "?? new.txt") {
		t.Fatalf("status = %q", status.Text)
	}
	if _, err := reg.Run(context.Background(), tools.Call{Name: "git_diff"}); err != nil {
		t.Fatalf("git_diff: %v", err)
	}
}
