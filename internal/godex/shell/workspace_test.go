package shell

import (
	"bytes"
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex/tools"
	toolbuiltin "github.com/pandelisz/gode/internal/godex/tools/builtin"
)

func TestWorkspaceBuiltinReadFileAndAllowsOutsideReads(t *testing.T) {
	root := t.TempDir()
	outside := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "main.go"), []byte("package main\nfunc main() {}\n"), 0o600); err != nil {
		t.Fatalf("write workspace file: %v", err)
	}
	outsidePath := filepath.Join(outside, "note.txt")
	if err := os.WriteFile(outsidePath, []byte("outside note\n"), 0o600); err != nil {
		t.Fatalf("write outside file: %v", err)
	}

	reg := workspaceBuiltinRegistry(t, root)
	result, err := Runner{Builtins: reg}.Run(context.Background(), RunRequest{
		Command: "gode_read_file main.go 2 1",
		Dir:     root,
		Policy:  &Policy{AllowExternal: false},
	})
	if err != nil {
		t.Fatalf("run read: %v", err)
	}
	if result.ExitCode != 0 || !strings.Contains(result.Stdout, "func main() {}") || strings.Contains(result.Stdout, "package main") {
		t.Fatalf("read result = %#v", result)
	}

	result, err = Runner{Builtins: reg}.Run(context.Background(), RunRequest{
		Command: "gode_read_file " + shellQuoteForTest(outsidePath),
		Dir:     root,
		Policy:  &Policy{AllowExternal: false},
	})
	if err != nil {
		t.Fatalf("run outside read: %v", err)
	}
	if result.ExitCode != 0 || !strings.Contains(result.Stdout, "outside note") {
		t.Fatalf("outside read result = %#v", result)
	}
}

func TestWorkspaceBuiltinListFilesSortedAndRejectsEscapes(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "b.txt"), []byte("b\n"), 0o600); err != nil {
		t.Fatalf("write b: %v", err)
	}
	if err := os.WriteFile(filepath.Join(root, "a.txt"), []byte("a\n"), 0o600); err != nil {
		t.Fatalf("write a: %v", err)
	}

	reg := workspaceBuiltinRegistry(t, root)
	result, err := Runner{Builtins: reg}.Run(context.Background(), RunRequest{
		Command: "gode_list_files .",
		Dir:     root,
		Policy:  &Policy{AllowExternal: false},
	})
	if err != nil {
		t.Fatalf("run list: %v", err)
	}
	if result.ExitCode != 0 || strings.TrimSpace(result.Stdout) != "a.txt\nb.txt" {
		t.Fatalf("list result = %#v", result)
	}

	result, err = Runner{Builtins: reg}.Run(context.Background(), RunRequest{
		Command: "gode_list_files ..",
		Dir:     root,
		Policy:  &Policy{AllowExternal: false},
	})
	if err != nil {
		t.Fatalf("run list escape: %v", err)
	}
	if result.ExitCode == 0 || !strings.Contains(result.Stderr, "path escapes workspace") {
		t.Fatalf("list escape result = %#v", result)
	}
}

func TestWorkspaceBuiltinSearchFilesSkipsBinary(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "text.txt"), []byte("needle in text\n"), 0o600); err != nil {
		t.Fatalf("write text: %v", err)
	}
	if err := os.WriteFile(filepath.Join(root, "binary.bin"), []byte{'n', 'e', 'e', 'd', 'l', 'e', 0, 1}, 0o600); err != nil {
		t.Fatalf("write binary: %v", err)
	}

	reg := workspaceBuiltinRegistry(t, root)
	result, err := Runner{Builtins: reg}.Run(context.Background(), RunRequest{
		Command: "gode_search_files needle",
		Dir:     root,
		Policy:  &Policy{AllowExternal: false},
	})
	if err != nil {
		t.Fatalf("run search: %v", err)
	}
	if result.ExitCode != 0 || !strings.Contains(result.Stdout, "text.txt:1:needle in text") || strings.Contains(result.Stdout, "binary.bin") {
		t.Fatalf("search result = %#v", result)
	}
}

func TestWorkspaceBuiltinApplyPatchUsesPatchTool(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "main.txt"), []byte("old\n"), 0o600); err != nil {
		t.Fatalf("write file: %v", err)
	}
	patch := strings.Join([]string{
		"*** Begin Patch",
		"*** Update File: main.txt",
		"@@",
		"-old",
		"+new",
		"*** End Patch",
		"",
	}, "\n")
	if err := os.WriteFile(filepath.Join(root, "change.patch"), []byte(patch), 0o600); err != nil {
		t.Fatalf("write patch: %v", err)
	}

	reg := workspaceBuiltinRegistry(t, root)
	result, err := Runner{Builtins: reg}.Run(context.Background(), RunRequest{
		Command: "gode_apply_patch < change.patch",
		Dir:     root,
		Policy:  &Policy{AllowExternal: false},
	})
	if err != nil {
		t.Fatalf("run patch: %v", err)
	}
	if result.ExitCode != 0 || !strings.Contains(result.Stdout, "Updated main.txt") {
		t.Fatalf("patch result = %#v", result)
	}
	data, err := os.ReadFile(filepath.Join(root, "main.txt"))
	if err != nil {
		t.Fatalf("read patched file: %v", err)
	}
	if string(data) != "new\n" {
		t.Fatalf("patched file = %q", data)
	}
}

func TestWorkspaceBuiltinsReturnContextCancellation(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "main.txt"), []byte("hello\n"), 0o600); err != nil {
		t.Fatalf("write file: %v", err)
	}
	toolReg := tools.NewRegistry()
	toolbuiltin.RegisterFilesystem(toolReg, root)
	toolbuiltin.RegisterPatch(toolReg, root)

	ctx, cancel := context.WithCancel(context.Background())
	cancel()

	var stdout bytes.Buffer
	var stderr bytes.Buffer
	checks := map[string]func() error{
		"gode_read_file": func() error {
			return handleWorkspaceReadFile(ctx, root, toolReg, []string{"gode_read_file", "main.txt"}, &stdout, &stderr)
		},
		"gode_list_files": func() error {
			return handleWorkspaceListFiles(ctx, root, toolReg, []string{"gode_list_files", "."}, &stdout, &stderr)
		},
		"gode_search_files": func() error {
			return handleWorkspaceSearchFiles(ctx, toolReg, []string{"gode_search_files", "hello"}, &stdout, &stderr)
		},
		"gode_apply_patch": func() error {
			return handleWorkspaceApplyPatch(ctx, toolReg, strings.NewReader("*** Begin Patch\n*** End Patch\n"), &stdout, &stderr)
		},
	}
	for name, check := range checks {
		if err := check(); !errors.Is(err, context.Canceled) {
			t.Fatalf("%s err = %v", name, err)
		}
	}
}

func workspaceBuiltinRegistry(t *testing.T, root string) *BuiltinRegistry {
	t.Helper()
	toolReg := tools.NewRegistry()
	toolbuiltin.RegisterFilesystem(toolReg, root)
	toolbuiltin.RegisterPatch(toolReg, root)
	reg := NewBuiltinRegistry()
	if err := RegisterWorkspaceBuiltins(reg, root, toolReg); err != nil {
		t.Fatalf("register workspace builtins: %v", err)
	}
	return reg
}

func shellQuoteForTest(path string) string {
	return "'" + strings.ReplaceAll(path, "'", "'\\''") + "'"
}
