package builtin

import (
	"context"
	"os/exec"
	"strings"
	"time"

	"github.com/pandelisz/gode/internal/godex/tools"
)

func RegisterShell(reg *tools.Registry, root string) {
	reg.Register(tools.Tool{
		Name:        "shell",
		Description: "Run a shell command in the workspace.",
		ReadOnly:    false,
		Schema:      objectSchema("command"),
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			command := stringInput(call.Input, "command")
			timeoutCtx, cancel := context.WithTimeout(ctx, 2*time.Minute)
			defer cancel()
			cmd := exec.CommandContext(timeoutCtx, "/bin/sh", "-lc", command)
			cmd.Dir = root
			out, err := cmd.CombinedOutput()
			text := strings.TrimRight(string(out), "\n")
			if err != nil {
				return tools.Result{Text: text}, err
			}
			return tools.Result{Text: text}, nil
		},
	})
}
