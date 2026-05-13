package builtin

import (
	"context"
	"os"
	"strings"
	"testing"
	"time"

	sdkmcp "github.com/modelcontextprotocol/go-sdk/mcp"
	godexmcp "github.com/pandelisz/gode/internal/godex/mcp"
	"github.com/pandelisz/gode/internal/godex/tools"
)

func TestMCPResourceAndPromptTools(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	manager := godexmcp.NewManager(nil, map[string]godexmcp.ServerConfig{"helper": {
		Type:    "stdio",
		Command: os.Args[0],
		Args:    []string{"-test.run=TestMCPBuiltinHelperProcess", "--"},
		Env:     map[string]string{"GODE_MCP_BUILTIN_HELPER": "1"},
		Timeout: 5,
	}})
	if err := manager.Start(ctx); err != nil {
		t.Fatalf("start: %v", err)
	}
	defer manager.Close(context.Background())
	reg := tools.NewRegistry(tools.WithAutoApprove(true))
	RegisterMCP(reg, manager)

	list, err := reg.Run(ctx, tools.Call{Name: "list_mcp_resources"})
	if err != nil {
		t.Fatalf("list resources: %v", err)
	}
	if !strings.Contains(list.Text, "file:///notes.txt") {
		t.Fatalf("resources = %s", list.Text)
	}
	read, err := reg.Run(ctx, tools.Call{Name: "read_mcp_resource", Input: map[string]any{"server": "helper", "uri": "file:///notes.txt"}})
	if err != nil {
		t.Fatalf("read resource: %v", err)
	}
	if read.Text != "resource notes" {
		t.Fatalf("read = %q", read.Text)
	}
	prompts, err := reg.Run(ctx, tools.Call{Name: "list_mcp_prompts"})
	if err != nil {
		t.Fatalf("list prompts: %v", err)
	}
	if !strings.Contains(prompts.Text, "greet") {
		t.Fatalf("prompts = %s", prompts.Text)
	}
	prompt, err := reg.Run(ctx, tools.Call{Name: "run_mcp_prompt", Input: map[string]any{"server": "helper", "name": "greet", "arguments": map[string]any{"name": "Pat"}}})
	if err != nil {
		t.Fatalf("run prompt: %v", err)
	}
	if !strings.Contains(prompt.Text, "assistant: hello Pat") {
		t.Fatalf("prompt = %q", prompt.Text)
	}
}

func TestMCPBuiltinHelperProcess(t *testing.T) {
	if os.Getenv("GODE_MCP_BUILTIN_HELPER") != "1" {
		return
	}
	server := sdkmcp.NewServer(&sdkmcp.Implementation{Name: "helper", Version: "test"}, nil)
	server.AddResource(&sdkmcp.Resource{URI: "file:///notes.txt", Name: "notes", MIMEType: "text/plain"}, func(_ context.Context, _ *sdkmcp.ReadResourceRequest) (*sdkmcp.ReadResourceResult, error) {
		return &sdkmcp.ReadResourceResult{Contents: []*sdkmcp.ResourceContents{{URI: "file:///notes.txt", MIMEType: "text/plain", Text: "resource notes"}}}, nil
	})
	server.AddPrompt(&sdkmcp.Prompt{Name: "greet", Description: "greet by name", Arguments: []*sdkmcp.PromptArgument{{Name: "name", Required: true}}}, func(_ context.Context, req *sdkmcp.GetPromptRequest) (*sdkmcp.GetPromptResult, error) {
		name := req.Params.Arguments["name"]
		return &sdkmcp.GetPromptResult{Messages: []*sdkmcp.PromptMessage{{Role: sdkmcp.Role("assistant"), Content: &sdkmcp.TextContent{Text: "hello " + name}}}}, nil
	})
	if err := server.Run(context.Background(), &sdkmcp.StdioTransport{}); err != nil {
		os.Exit(1)
	}
	os.Exit(0)
}
