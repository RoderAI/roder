package godex

import (
	"context"
	"fmt"
	"os"
	"path/filepath"

	"github.com/google/uuid"
	"github.com/pandelisz/gode/internal/godex/agent"
	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/journal"
	"github.com/pandelisz/gode/internal/godex/mcp"
	"github.com/pandelisz/gode/internal/godex/provider"
	"github.com/pandelisz/gode/internal/godex/tools"
	"github.com/pandelisz/gode/internal/godex/tools/builtin"
)

type App struct {
	Config  Config
	Bus     *eventbus.Bus
	Journal *journal.Store
	Tools   *tools.Registry
	MCP     *mcp.Manager

	provider provider.Provider
	runner   *agent.Runner
}

func New(ctx context.Context, cfg Config) (*App, error) {
	cfg = cfg.withDefaults()
	if err := os.MkdirAll(cfg.Workspace, 0o755); err != nil {
		return nil, fmt.Errorf("workspace: %w", err)
	}
	if err := os.MkdirAll(cfg.DataDir, 0o700); err != nil {
		return nil, fmt.Errorf("data dir: %w", err)
	}

	bus := eventbus.New(eventbus.WithSubscriberBuffer(4096))
	store, err := journal.Open(filepath.Join(cfg.DataDir, "events.jsonl"))
	if err != nil {
		return nil, err
	}

	reg := tools.NewRegistry(tools.WithEventBus(bus), tools.WithAutoApprove(cfg.AutoApprove))
	builtin.RegisterFilesystem(reg, cfg.Workspace)
	builtin.RegisterTodo(reg)
	builtin.RegisterMemory(reg, filepath.Join(cfg.DataDir, "memory.jsonl"))
	builtin.RegisterShell(reg, cfg.Workspace)
	builtin.RegisterPatch(reg, cfg.Workspace)
	builtin.RegisterSubagent(reg)

	mcpManager := mcp.NewManager(bus, cfg.MCP)
	_ = mcpManager.Start(ctx)
	builtin.RegisterMCP(reg, mcpManager)

	prov, err := buildProvider(cfg)
	if err != nil {
		store.Close()
		return nil, err
	}
	runner := agent.NewRunner(agent.Config{Bus: bus, Journal: store, Tools: reg, Provider: prov})

	return &App{
		Config:   cfg,
		Bus:      bus,
		Journal:  store,
		Tools:    reg,
		MCP:      mcpManager,
		provider: prov,
		runner:   runner,
	}, nil
}

func (a *App) RunPrompt(ctx context.Context, prompt string) (agent.RunResult, error) {
	return a.runner.Run(ctx, agent.RunRequest{SessionID: uuid.NewString(), Prompt: prompt})
}

func (a *App) Close(ctx context.Context) error {
	if a.MCP != nil {
		_ = a.MCP.Close(ctx)
	}
	if a.Journal != nil {
		_ = a.Journal.Flush()
		_ = a.Journal.Close()
	}
	if a.Bus != nil {
		_ = a.Bus.Close()
	}
	return nil
}

func buildProvider(cfg Config) (provider.Provider, error) {
	switch cfg.Provider {
	case "mock":
		return provider.NewMock("mock response", nil), nil
	case "codex", "openai":
		return provider.NewOpenAI(cfg.Model), nil
	default:
		return nil, fmt.Errorf("unknown provider %q", cfg.Provider)
	}
}
