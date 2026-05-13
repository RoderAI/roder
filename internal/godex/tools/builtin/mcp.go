package builtin

import (
	"context"
	"encoding/json"
	"fmt"
	"unicode"

	godexmcp "github.com/pandelisz/gode/internal/godex/mcp"
	"github.com/pandelisz/gode/internal/godex/tools"
)

func RegisterMCP(reg *tools.Registry, manager *godexmcp.Manager) {
	if manager == nil {
		return
	}
	registerMCPResources(reg, manager)
	registerMCPPrompts(reg, manager)
	for _, remote := range manager.Tools() {
		remote := remote
		name := "mcp_" + safeToolName(remote.Server) + "_" + safeToolName(remote.Name)
		reg.Register(tools.Tool{
			Name:        name,
			Description: remote.Description,
			Schema:      remote.InputSchema,
			ReadOnly:    false,
			Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
				text, err := manager.CallTool(ctx, remote.Server, remote.Name, call.Input)
				return tools.Result{Text: text}, err
			},
		})
	}
}

func safeToolName(value string) string {
	var out []rune
	for _, r := range value {
		if unicode.IsLetter(r) || unicode.IsDigit(r) || r == '_' {
			out = append(out, r)
		} else {
			out = append(out, '_')
		}
	}
	if len(out) == 0 {
		return "unnamed"
	}
	return string(out)
}

func registerMCPResources(reg *tools.Registry, manager *godexmcp.Manager) {
	reg.Register(tools.Tool{
		Name:        "list_mcp_resources",
		Description: "List resources exposed by connected MCP servers.",
		ReadOnly:    true,
		Schema:      objectSchema(),
		Run: func(context.Context, tools.Call) (tools.Result, error) {
			data, _ := json.MarshalIndent(manager.Resources(), "", "  ")
			return tools.Result{Text: string(data)}, nil
		},
	})
	reg.Register(tools.Tool{
		Name:        "read_mcp_resource",
		Description: "Read a resource exposed by a connected MCP server.",
		ReadOnly:    true,
		Schema:      objectSchema("server", "uri"),
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			server := stringInput(call.Input, "server")
			uri := stringInput(call.Input, "uri")
			if server == "" || uri == "" {
				return tools.Result{}, fmt.Errorf("server and uri are required")
			}
			text, err := manager.ReadResource(ctx, server, uri)
			return tools.Result{Text: text}, err
		},
	})
}

func registerMCPPrompts(reg *tools.Registry, manager *godexmcp.Manager) {
	reg.Register(tools.Tool{
		Name:        "list_mcp_prompts",
		Description: "List prompts exposed by connected MCP servers.",
		ReadOnly:    true,
		Schema:      objectSchema(),
		Run: func(context.Context, tools.Call) (tools.Result, error) {
			data, _ := json.MarshalIndent(manager.Prompts(), "", "  ")
			return tools.Result{Text: string(data)}, nil
		},
	})
	reg.Register(tools.Tool{
		Name:        "run_mcp_prompt",
		Description: "Run a prompt exposed by a connected MCP server.",
		ReadOnly:    true,
		Schema:      objectSchema("server", "name"),
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			server := stringInput(call.Input, "server")
			name := stringInput(call.Input, "name")
			if server == "" || name == "" {
				return tools.Result{}, fmt.Errorf("server and name are required")
			}
			text, err := manager.GetPrompt(ctx, server, name, stringMapInput(call.Input, "arguments"))
			return tools.Result{Text: text}, err
		},
	})
}

func stringMapInput(input map[string]any, key string) map[string]string {
	value, ok := input[key]
	if !ok || value == nil {
		return nil
	}
	out := map[string]string{}
	switch typed := value.(type) {
	case map[string]string:
		return typed
	case map[string]any:
		for key, value := range typed {
			if text, ok := value.(string); ok {
				out[key] = text
			}
		}
	}
	return out
}
