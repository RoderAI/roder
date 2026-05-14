package builtin

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex/tools"
	"github.com/pandelisz/gode/internal/godex/workspacepath"
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
	if !strings.Contains(read.Text, "path: main.go") || !strings.Contains(read.Text, "lines: 1-2 of 2") {
		t.Fatalf("read text = %q", read.Text)
	}
	if !strings.Contains(read.Text, "     1 | package main") || !strings.Contains(read.Text, "     2 | func main() {}") {
		t.Fatalf("read text should include numbered content, got %q", read.Text)
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

func TestReadFileRequiresFocusedLineRange(t *testing.T) {
	root := t.TempDir()
	var content strings.Builder
	for i := 1; i <= 450; i++ {
		content.WriteString(fmt.Sprintf("line %03d\n", i))
	}
	if err := os.WriteFile(filepath.Join(root, "large.txt"), []byte(content.String()), 0o600); err != nil {
		t.Fatalf("write: %v", err)
	}
	reg := tools.NewRegistry()
	RegisterFilesystem(reg, root)

	read, err := reg.Run(context.Background(), tools.Call{Name: "read_file", Input: map[string]any{
		"path":       "large.txt",
		"start_line": 10,
		"limit":      3,
	}})
	if err != nil {
		t.Fatalf("read range: %v", err)
	}
	if !strings.Contains(read.Text, "lines: 10-12 of 450") {
		t.Fatalf("read range should report requested lines, got:\n%s", read.Text)
	}
	if strings.Contains(read.Text, "     9 |") || strings.Contains(read.Text, "    13 |") {
		t.Fatalf("read range leaked lines outside requested range:\n%s", read.Text)
	}

	defaultRead, err := reg.Run(context.Background(), tools.Call{Name: "read_file", Input: map[string]any{"path": "large.txt"}})
	if err != nil {
		t.Fatalf("read default range: %v", err)
	}
	if !strings.Contains(defaultRead.Text, "lines: 1-200 of 450") || !strings.Contains(defaultRead.Text, "truncated: true") || !strings.Contains(defaultRead.Text, "next_start_line: 201") {
		t.Fatalf("default read should be capped with continuation hint, got:\n%s", defaultRead.Text)
	}
	if strings.Contains(defaultRead.Text, "   201 |") {
		t.Fatalf("default read should not include line 201:\n%s", defaultRead.Text)
	}

	clamped, err := reg.Run(context.Background(), tools.Call{Name: "read_file", Input: map[string]any{
		"path":  "large.txt",
		"limit": 999,
	}})
	if err != nil {
		t.Fatalf("read clamped range: %v", err)
	}
	if !strings.Contains(clamped.Text, "lines: 1-400 of 450") || !strings.Contains(clamped.Text, "max_line_limit: 400") {
		t.Fatalf("read should clamp large limits, got:\n%s", clamped.Text)
	}
}

func TestReadFileRejectsInvalidRange(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "main.go"), []byte("package main\n"), 0o600); err != nil {
		t.Fatalf("write: %v", err)
	}
	reg := tools.NewRegistry()
	RegisterFilesystem(reg, root)

	result, err := reg.Run(context.Background(), tools.Call{Name: "read_file", Input: map[string]any{
		"path":       "main.go",
		"start_line": 0,
	}})
	if err != nil {
		t.Fatalf("run should return invalid range as a failed tool result: %v", err)
	}
	if result.Error == "" || !strings.Contains(result.Text, "start_line must be >= 1") {
		t.Fatalf("failed result = %#v", result)
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

func TestReadFileAllowsPathsOutsideWorkspace(t *testing.T) {
	root := t.TempDir()
	outside := t.TempDir()
	outsidePath := filepath.Join(outside, "secret.txt")
	if err := os.WriteFile(outsidePath, []byte("outside workspace\n"), 0o600); err != nil {
		t.Fatalf("write outside: %v", err)
	}
	reg := tools.NewRegistry()
	RegisterFilesystem(reg, root)

	relative, err := filepath.Rel(root, outsidePath)
	if err != nil {
		t.Fatalf("rel: %v", err)
	}
	result, err := reg.Run(context.Background(), tools.Call{Name: "read_file", Input: map[string]any{"path": relative}})
	if err != nil {
		t.Fatalf("read relative outside workspace: %v", err)
	}
	if result.Error != "" || !strings.Contains(result.Text, "outside workspace") || !strings.Contains(result.Text, "path: "+relative) {
		t.Fatalf("read relative outside workspace result = %#v", result)
	}

	result, err = reg.Run(context.Background(), tools.Call{Name: "read_file", Input: map[string]any{"path": outsidePath}})
	if err != nil {
		t.Fatalf("read absolute outside workspace: %v", err)
	}
	if result.Error != "" || !strings.Contains(result.Text, "outside workspace") || !strings.Contains(result.Text, "path: "+outsidePath) {
		t.Fatalf("read absolute outside workspace result = %#v", result)
	}
}

func TestCleanWorkspacePathRejectsEscapeAndReadPathAllowsOutside(t *testing.T) {
	root := t.TempDir()
	if _, err := workspacepath.CleanWorkspacePath(root, "../outside.txt"); err == nil {
		t.Fatal("expected workspace path escape to be rejected")
	}

	outside := t.TempDir()
	outsidePath := filepath.Join(outside, "note.txt")
	readPath, err := workspacepath.CleanReadPath(root, outsidePath)
	if err != nil {
		t.Fatalf("clean read path: %v", err)
	}
	if readPath != outsidePath {
		t.Fatalf("read path = %q, want %q", readPath, outsidePath)
	}
}

func TestWriteFileRejectsPathEscape(t *testing.T) {
	root := t.TempDir()
	reg := tools.NewRegistry()
	RegisterEditing(reg, root)

	result, err := reg.Run(context.Background(), tools.Call{Name: "write_file", Input: map[string]any{"path": "../secret", "content": "nope"}})
	if err != nil {
		t.Fatalf("run should return path escape as a failed tool result: %v", err)
	}
	if result.Error == "" || !strings.Contains(result.Text, "path escapes workspace") {
		t.Fatalf("failed result = %#v", result)
	}
}

func runGit(t *testing.T, dir string, args ...string) {
	t.Helper()
	cmd := exec.Command("git", append([]string{"-C", dir}, args...)...)
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("git %v: %v\n%s", args, err, out)
	}
}
