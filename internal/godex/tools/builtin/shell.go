package builtin

import (
	"context"
	"fmt"
	"strings"
	"time"

	"github.com/pandelisz/gode/internal/godex/permission"
	godexshell "github.com/pandelisz/gode/internal/godex/shell"
	"github.com/pandelisz/gode/internal/godex/tools"
)

type ShellRunner interface {
	Run(context.Context, godexshell.RunRequest) (godexshell.RunResult, error)
}

func RegisterShell(reg *tools.Registry, root string, runner ShellRunner) {
	if runner == nil {
		runner = godexshell.NewRunner()
	}
	reg.Register(tools.Tool{
		Name:        "shell",
		Description: "Run a POSIX shell command in the workspace.",
		ReadOnly:    false,
		Action:      permission.ActionExecute,
		Schema:      objectSchema("command"),
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			command := stringInput(call.Input, "command")
			result, err := runner.Run(ctx, godexshell.RunRequest{
				Command:       command,
				Dir:           root,
				Timeout:       2 * time.Minute,
				CombineOutput: true,
			})
			text := strings.TrimRight(result.Stdout, "\n")
			if err != nil {
				return tools.Result{Text: text}, err
			}
			if result.ExitCode != 0 {
				return tools.Result{Text: text}, fmt.Errorf("shell exited with status %d", result.ExitCode)
			}
			return tools.Result{Text: text}, nil
		},
	})
}
