package builtin

import (
	"context"
	"encoding/json"
	"fmt"

	"github.com/pandelisz/gode/internal/godex/lsp"
	"github.com/pandelisz/gode/internal/godex/permission"
	"github.com/pandelisz/gode/internal/godex/tools"
)

func RegisterLSP(reg *tools.Registry, manager *lsp.Manager) {
	if manager == nil {
		return
	}
	reg.Register(tools.Tool{
		Name:        "lsp_diagnostics",
		Description: "Return diagnostics reported by configured language servers for a file path.",
		ReadOnly:    true,
		Action:      permission.ActionRead,
		Schema:      objectSchema("path"),
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			path := stringInput(call.Input, "path")
			if path == "" {
				return tools.Result{}, fmt.Errorf("path is required")
			}
			diagnostics, err := manager.Diagnostics(ctx, path)
			if err != nil {
				return tools.Result{}, err
			}
			data, _ := json.MarshalIndent(diagnostics, "", "  ")
			return tools.Result{Text: string(data)}, nil
		},
	})
	reg.Register(tools.Tool{
		Name:        "lsp_restart",
		Description: "Restart a configured language server.",
		ReadOnly:    false,
		Action:      permission.ActionExecute,
		Schema:      objectSchema("server"),
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			server := stringInput(call.Input, "server")
			if server == "" {
				return tools.Result{}, fmt.Errorf("server is required")
			}
			if err := manager.Restart(ctx, server); err != nil {
				return tools.Result{}, err
			}
			return tools.Result{Text: "restarted " + server}, nil
		},
	})
}
