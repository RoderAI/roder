package builtin

import (
	"context"
	"strings"

	godexmcp "github.com/pandelisz/gode/internal/godex/mcp"
	"github.com/pandelisz/gode/internal/godex/tools"
)

func RegisterMCP(reg *tools.Registry, manager *godexmcp.Manager) {
	if manager == nil {
		return
	}
	for _, remote := range manager.Tools() {
		remote := remote
		name := "mcp." + remote.Server + "." + remote.Name
		reg.Register(tools.Tool{
			Name:        name,
			Description: remote.Description,
			Schema:      remote.InputSchema,
			ReadOnly:    false,
			Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
				parts := strings.SplitN(strings.TrimPrefix(call.Name, "mcp."), ".", 2)
				if len(parts) != 2 {
					parts = []string{remote.Server, remote.Name}
				}
				text, err := manager.CallTool(ctx, parts[0], parts[1], call.Input)
				return tools.Result{Text: text}, err
			},
		})
	}
}
