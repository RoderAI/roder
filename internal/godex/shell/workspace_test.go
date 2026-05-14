package shell_test

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"testing"

	godexshell "github.com/pandelisz/gode/internal/godex/shell"
	"github.com/pandelisz/gode/internal/godex/tools"
	"github.com/pandelisz/gode/internal/godex/workspacepath"
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
	result, err := godexshell.Runner{Builtins: reg}.Run(context.Background(), godexshell.RunRequest{
		Command: "gode_read_file main.go 2 1",
		Dir:     root,
		Policy:  &godexshell.Policy{AllowExternal: false},
	})
	if err != nil {
		t.Fatalf("run read: %v", err)
	}
	if result.ExitCode != 0 || !strings.Contains(result.Stdout, "func main() {}") || strings.Contains(result.Stdout, "package main") {
		t.Fatalf("read result = %#v", result)
	}

	result, err = godexshell.Runner{Builtins: reg}.Run(context.Background(), godexshell.RunRequest{
		Command: "gode_read_file " + shellQuoteForTest(outsidePath),
		Dir:     root,
		Policy:  &godexshell.Policy{AllowExternal: false},
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
	result, err := godexshell.Runner{Builtins: reg}.Run(context.Background(), godexshell.RunRequest{
		Command: "gode_list_files .",
		Dir:     root,
		Policy:  &godexshell.Policy{AllowExternal: false},
	})
	if err != nil {
		t.Fatalf("run list: %v", err)
	}
	if result.ExitCode != 0 || strings.TrimSpace(result.Stdout) != "a.txt\nb.txt" {
		t.Fatalf("list result = %#v", result)
	}

	result, err = godexshell.Runner{Builtins: reg}.Run(context.Background(), godexshell.RunRequest{
		Command: "gode_list_files ..",
		Dir:     root,
		Policy:  &godexshell.Policy{AllowExternal: false},
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
	result, err := godexshell.Runner{Builtins: reg}.Run(context.Background(), godexshell.RunRequest{
		Command: "gode_search_files needle",
		Dir:     root,
		Policy:  &godexshell.Policy{AllowExternal: false},
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
	result, err := godexshell.Runner{Builtins: reg}.Run(context.Background(), godexshell.RunRequest{
		Command: "gode_apply_patch < change.patch",
		Dir:     root,
		Policy:  &godexshell.Policy{AllowExternal: false},
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

func TestWorkspaceBuiltinApplyPatchOnlyRegistersWhenPatchToolExists(t *testing.T) {
	root := t.TempDir()
	toolReg := tools.NewRegistry()
	registerFakeWorkspaceReadTools(t, toolReg, root)

	reg := godexshell.NewBuiltinRegistry()
	if err := godexshell.RegisterWorkspaceBuiltins(reg, root, toolReg); err != nil {
		t.Fatalf("register workspace builtins: %v", err)
	}
	if _, ok := reg.Lookup("gode_apply_patch"); ok {
		t.Fatalf("gode_apply_patch should not be registered without apply_patch tool")
	}

	toolReg.Register(tools.Tool{Name: "apply_patch", Run: func(context.Context, tools.Call) (tools.Result, error) {
		return tools.Result{Text: "patched"}, nil
	}})
	reg = godexshell.NewBuiltinRegistry()
	if err := godexshell.RegisterWorkspaceBuiltins(reg, root, toolReg); err != nil {
		t.Fatalf("register workspace builtins with patch: %v", err)
	}
	if _, ok := reg.Lookup("gode_apply_patch"); !ok {
		t.Fatalf("gode_apply_patch should be registered when apply_patch tool exists")
	}
}

func TestWorkspaceBuiltinsWithoutPatchStillReturnContextCancellation(t *testing.T) {
	root := t.TempDir()
	toolReg := tools.NewRegistry()
	registerFakeWorkspaceReadTools(t, toolReg, root)
	reg := godexshell.NewBuiltinRegistry()
	if err := godexshell.RegisterWorkspaceBuiltins(reg, root, toolReg); err != nil {
		t.Fatalf("register workspace builtins: %v", err)
	}
	if _, ok := reg.Lookup("gode_apply_patch"); ok {
		t.Fatalf("gode_apply_patch should not be registered")
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	for _, name := range []string{"gode_read_file", "gode_list_files", "gode_search_files"} {
		builtin, ok := reg.Lookup(name)
		if !ok {
			t.Fatalf("missing builtin %s", name)
		}
		err := builtin.Run(ctx, []string{name, "main.txt"}, strings.NewReader(""), nilWriter{}, nilWriter{})
		if !errors.Is(err, context.Canceled) {
			t.Fatalf("%s err = %v", name, err)
		}
	}
}

func TestWorkspaceBuiltinsReturnContextCancellation(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "main.txt"), []byte("hello\n"), 0o600); err != nil {
		t.Fatalf("write file: %v", err)
	}
	reg := workspaceBuiltinRegistry(t, root)

	ctx, cancel := context.WithCancel(context.Background())
	cancel()

	checks := map[string][]string{
		"gode_read_file":    {"gode_read_file", "main.txt"},
		"gode_list_files":   {"gode_list_files", "."},
		"gode_search_files": {"gode_search_files", "hello"},
		"gode_apply_patch":  {"gode_apply_patch"},
	}
	for name, args := range checks {
		builtin, ok := reg.Lookup(name)
		if !ok {
			t.Fatalf("missing builtin %s", name)
		}
		err := builtin.Run(ctx, args, strings.NewReader("*** Begin Patch\n*** End Patch\n"), nilWriter{}, nilWriter{})
		if !errors.Is(err, context.Canceled) {
			t.Fatalf("%s err = %v", name, err)
		}
	}
}

func workspaceBuiltinRegistry(t *testing.T, root string) *godexshell.BuiltinRegistry {
	t.Helper()
	toolReg := tools.NewRegistry()
	registerFakeWorkspaceTools(t, toolReg, root)
	reg := godexshell.NewBuiltinRegistry()
	if err := godexshell.RegisterWorkspaceBuiltins(reg, root, toolReg); err != nil {
		t.Fatalf("register workspace builtins: %v", err)
	}
	return reg
}

func shellQuoteForTest(path string) string {
	return "'" + strings.ReplaceAll(path, "'", "'\\''") + "'"
}

func registerFakeWorkspaceTools(t *testing.T, reg *tools.Registry, root string) {
	t.Helper()
	registerFakeWorkspaceReadTools(t, reg, root)
	reg.Register(tools.Tool{
		Name:     "apply_patch",
		ReadOnly: false,
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			patch := stringValue(call.Input, "patch")
			if !strings.Contains(patch, "-old\n+new") {
				return tools.Result{}, fmt.Errorf("unexpected patch")
			}
			if err := os.WriteFile(filepath.Join(root, "main.txt"), []byte("new\n"), 0o600); err != nil {
				return tools.Result{}, err
			}
			return tools.Result{Text: "Updated main.txt"}, nil
		},
	})
}

func registerFakeWorkspaceReadTools(t *testing.T, reg *tools.Registry, root string) {
	t.Helper()
	reg.Register(tools.Tool{
		Name:     "read_file",
		ReadOnly: true,
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			path := stringValue(call.Input, "path")
			cleaned, err := workspacepath.CleanReadPath(root, path)
			if err != nil {
				return tools.Result{}, err
			}
			data, err := os.ReadFile(cleaned)
			if err != nil {
				return tools.Result{}, err
			}
			lines := strings.Split(strings.TrimRight(string(data), "\n"), "\n")
			start := intValue(call.Input, "start_line", 1)
			limit := intValue(call.Input, "limit", len(lines))
			if start < 1 {
				start = 1
			}
			from := start - 1
			if from > len(lines) {
				from = len(lines)
			}
			to := from + limit
			if to > len(lines) {
				to = len(lines)
			}
			return tools.Result{Text: strings.Join(lines[from:to], "\n")}, nil
		},
	})
	reg.Register(tools.Tool{
		Name:     "list_files",
		ReadOnly: true,
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			path, err := workspacepath.CleanWorkspacePath(root, stringValueDefault(call.Input, "path", "."))
			if err != nil {
				return tools.Result{}, err
			}
			entries, err := os.ReadDir(path)
			if err != nil {
				return tools.Result{}, err
			}
			names := make([]string, 0, len(entries))
			for _, entry := range entries {
				name := entry.Name()
				if entry.IsDir() {
					name += "/"
				}
				names = append(names, name)
			}
			sort.Strings(names)
			return tools.Result{Text: strings.Join(names, "\n")}, nil
		},
	})
	reg.Register(tools.Tool{
		Name:     "search_files",
		ReadOnly: true,
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			query := stringValue(call.Input, "query")
			var matches []string
			err := filepath.WalkDir(root, func(path string, entry os.DirEntry, err error) error {
				if err != nil {
					return err
				}
				if entry.IsDir() {
					return nil
				}
				data, err := os.ReadFile(path)
				if err != nil || strings.ContainsRune(string(data), '\x00') {
					return nil
				}
				rel, _ := filepath.Rel(root, path)
				for index, line := range strings.Split(strings.TrimRight(string(data), "\n"), "\n") {
					if strings.Contains(line, query) {
						matches = append(matches, fmt.Sprintf("%s:%d:%s", filepath.ToSlash(rel), index+1, line))
					}
				}
				return nil
			})
			if err != nil {
				return tools.Result{}, err
			}
			return tools.Result{Text: strings.Join(matches, "\n")}, nil
		},
	})
}

func stringValue(input map[string]any, key string) string {
	value, _ := input[key].(string)
	return value
}

func stringValueDefault(input map[string]any, key string, fallback string) string {
	if value := stringValue(input, key); value != "" {
		return value
	}
	return fallback
}

func intValue(input map[string]any, key string, fallback int) int {
	switch value := input[key].(type) {
	case int:
		return value
	case float64:
		return int(value)
	default:
		return fallback
	}
}

type nilWriter struct{}

func (nilWriter) Write(p []byte) (int, error) { return len(p), nil }
