package builtin

import (
	"context"
	"os"
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

func TestFilesystemToolsRejectPathEscape(t *testing.T) {
	root := t.TempDir()
	reg := tools.NewRegistry()
	RegisterFilesystem(reg, root)

	_, err := reg.Run(context.Background(), tools.Call{Name: "read_file", Input: map[string]any{"path": "../secret"}})
	if err == nil {
		t.Fatal("expected path escape error")
	}
}
