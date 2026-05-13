package builtin

import (
	"context"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex/tools"
)

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
