package builtin

import (
	"context"
	"os/exec"
	"strings"

	"github.com/pandelisz/gode/internal/godex/permission"
	"github.com/pandelisz/gode/internal/godex/tools"
)

func RegisterGit(reg *tools.Registry, root string) {
	reg.Register(tools.Tool{
		Name:        "git_status",
		Description: "Show git status for the workspace.",
		ReadOnly:    true,
		Action:      permission.ActionRead,
		Schema:      objectSchema(),
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			cmd := exec.CommandContext(ctx, "git", "status", "--short")
			cmd.Dir = root
			out, err := cmd.CombinedOutput()
			return tools.Result{Text: strings.TrimRight(string(out), "\n")}, err
		},
	})

	reg.Register(tools.Tool{
		Name:        "git_diff",
		Description: "Show git diff for the workspace.",
		ReadOnly:    true,
		Action:      permission.ActionRead,
		Schema:      objectSchema(),
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			args := []string{"diff"}
			if stringInput(call.Input, "cached") == "true" {
				args = append(args, "--cached")
			}
			cmd := exec.CommandContext(ctx, "git", args...)
			cmd.Dir = root
			out, err := cmd.CombinedOutput()
			return tools.Result{Text: strings.TrimRight(string(out), "\n")}, err
		},
	})
}
