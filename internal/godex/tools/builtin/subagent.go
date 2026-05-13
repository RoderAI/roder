package builtin

import (
	"context"

	"github.com/pandelisz/gode/internal/godex/tools"
)

func RegisterSubagent(reg *tools.Registry) {
	reg.Register(tools.Tool{
		Name:        "subagent",
		Description: "Record a subagent task request. Execution is intentionally deferred to a future worker runtime.",
		ReadOnly:    false,
		Schema:      objectSchema("task"),
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			return tools.Result{Text: "subagent requested: " + stringInput(call.Input, "task")}, nil
		},
	})
}
