package builtin

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex/tools"
)

func TestApplyPatchAcceptsCodexPatchUpdate(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "hello.txt"), []byte("hello\n"), 0o600); err != nil {
		t.Fatalf("write fixture: %v", err)
	}
	reg := tools.NewRegistry()
	RegisterPatch(reg, root)

	result, err := reg.Run(context.Background(), tools.Call{
		Name: "apply_patch",
		Input: map[string]any{"patch": `*** Begin Patch
*** Update File: hello.txt
@@
-hello
+world
*** End Patch
`},
	})
	if err != nil {
		t.Fatalf("apply patch: %v", err)
	}
	if result.Error != "" {
		t.Fatalf("unexpected tool error: %#v", result)
	}
	data, err := os.ReadFile(filepath.Join(root, "hello.txt"))
	if err != nil {
		t.Fatalf("read patched file: %v", err)
	}
	if string(data) != "world\n" {
		t.Fatalf("patched file = %q", string(data))
	}
	if !strings.Contains(result.Text, "Updated hello.txt") {
		t.Fatalf("result text = %q", result.Text)
	}
}

func TestApplyPatchAcceptsCodexPatchAddAndDelete(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "remove.txt"), []byte("bye\n"), 0o600); err != nil {
		t.Fatalf("write fixture: %v", err)
	}
	reg := tools.NewRegistry()
	RegisterPatch(reg, root)

	result, err := reg.Run(context.Background(), tools.Call{
		Name: "apply_patch",
		Input: map[string]any{"patch": `*** Begin Patch
*** Add File: added.txt
+new file
*** Delete File: remove.txt
*** End Patch
`},
	})
	if err != nil {
		t.Fatalf("apply patch: %v", err)
	}
	if result.Error != "" {
		t.Fatalf("unexpected tool error: %#v", result)
	}
	if data, err := os.ReadFile(filepath.Join(root, "added.txt")); err != nil || string(data) != "new file\n" {
		t.Fatalf("added file = %q, err=%v", string(data), err)
	}
	if _, err := os.Stat(filepath.Join(root, "remove.txt")); !os.IsNotExist(err) {
		t.Fatalf("removed file stat err = %v", err)
	}
}

func TestApplyPatchCodexFailureIncludesDiagnostic(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "hello.txt"), []byte("hello\n"), 0o600); err != nil {
		t.Fatalf("write fixture: %v", err)
	}
	reg := tools.NewRegistry()
	RegisterPatch(reg, root)

	result, err := reg.Run(context.Background(), tools.Call{
		Name: "apply_patch",
		Input: map[string]any{"patch": `*** Begin Patch
*** Update File: hello.txt
@@
-missing
+world
*** End Patch
`},
	})
	if err != nil {
		t.Fatalf("run should return apply_patch failure as tool output: %v", err)
	}
	if result.Error == "" {
		t.Fatal("expected tool failure marker")
	}
	for _, want := range []string{"failed to apply patch", "expected hunk not found", "hello.txt"} {
		if !strings.Contains(result.Text, want) {
			t.Fatalf("result text missing %q:\n%s", want, result.Text)
		}
	}
}

func TestApplyPatchFailureIncludesGitOutput(t *testing.T) {
	reg := tools.NewRegistry()
	RegisterPatch(reg, t.TempDir())

	result, err := reg.Run(context.Background(), tools.Call{
		Name:  "apply_patch",
		Input: map[string]any{"patch": "not a patch"},
	})
	if err != nil {
		t.Fatalf("run should return apply_patch failure as tool output: %v", err)
	}
	if result.Error == "" {
		t.Fatal("expected tool failure marker")
	}
	for _, want := range []string{"failed to apply patch", "exit status 128", "error:"} {
		if !strings.Contains(result.Text, want) {
			t.Fatalf("result text missing %q:\n%s", want, result.Text)
		}
	}
}
