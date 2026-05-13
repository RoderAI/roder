package builtin

import (
	"context"
	"fmt"
	"os/exec"
	"strings"

	"github.com/pandelisz/gode/internal/godex/permission"
	"github.com/pandelisz/gode/internal/godex/tools"
)

func RegisterPatch(reg *tools.Registry, root string) {
	reg.Register(tools.Tool{
		Name:        "apply_patch",
		Description: "Apply a unified patch in the workspace using git apply.",
		ReadOnly:    false,
		Action:      permission.ActionWrite,
		Schema:      objectSchema("patch"),
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			patch := stringInput(call.Input, "patch")
			cmd := exec.CommandContext(ctx, "git", "apply", "--whitespace=nowarn", "-")
			cmd.Dir = root
			cmd.Stdin = strings.NewReader(patch)
			out, err := cmd.CombinedOutput()
			result := tools.Result{Text: strings.TrimSpace(string(out))}
			if err != nil {
				return result, fmt.Errorf("failed to apply patch: %w", err)
			}
			return result, nil
		},
	})
}
