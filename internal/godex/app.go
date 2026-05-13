package godex

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/google/uuid"
	"github.com/pandelisz/gode/internal/godex/agent"
	"github.com/pandelisz/gode/internal/godex/codexauth"
	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/journal"
	"github.com/pandelisz/gode/internal/godex/mcp"
	"github.com/pandelisz/gode/internal/godex/provider"
	godetelemetry "github.com/pandelisz/gode/internal/godex/telemetry"
	"github.com/pandelisz/gode/internal/godex/tools"
	"github.com/pandelisz/gode/internal/godex/tools/builtin"
	"go.opentelemetry.io/otel"
	"go.opentelemetry.io/otel/attribute"
	"go.opentelemetry.io/otel/codes"
	"go.opentelemetry.io/otel/trace"
)

type App struct {
	Config  Config
	Bus     *eventbus.Bus
	Journal *journal.Store
	Tools   *tools.Registry
	MCP     *mcp.Manager

	provider          provider.Provider
	runner            *agent.Runner
	shutdownTelemetry func(context.Context) error
}

var tracer = otel.Tracer("github.com/pandelisz/gode/internal/godex")

func New(ctx context.Context, cfg Config) (*App, error) {
	cfg = cfg.withDefaults()
	shutdownTelemetry, err := godetelemetry.Setup(ctx, godetelemetry.Config{
		Enabled:     cfg.Telemetry,
		Endpoint:    cfg.TelemetryEndpoint,
		ServiceName: "gode",
	})
	if err != nil {
		return nil, fmt.Errorf("telemetry: %w", err)
	}
	ctx, span := tracer.Start(ctx, "godex.new",
		trace.WithAttributes(
			attribute.String("gode.provider", cfg.Provider),
			attribute.String("gode.model", cfg.Model),
			attribute.Bool("gode.telemetry.enabled", cfg.Telemetry),
		),
	)
	defer span.End()
	if err := os.MkdirAll(cfg.Workspace, 0o755); err != nil {
		recordSpanError(span, err)
		_ = shutdownTelemetry(ctx)
		return nil, fmt.Errorf("workspace: %w", err)
	}
	if err := os.MkdirAll(cfg.DataDir, 0o700); err != nil {
		recordSpanError(span, err)
		_ = shutdownTelemetry(ctx)
		return nil, fmt.Errorf("data dir: %w", err)
	}

	bus := eventbus.New(eventbus.WithSubscriberBuffer(4096))
	store, err := journal.Open(filepath.Join(cfg.DataDir, "events.jsonl"))
	if err != nil {
		recordSpanError(span, err)
		_ = shutdownTelemetry(ctx)
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
		recordSpanError(span, err)
		_ = shutdownTelemetry(ctx)
		return nil, err
	}
	runner := agent.NewRunner(agent.Config{Bus: bus, Journal: store, Tools: reg, Provider: prov})

	return &App{
		Config:            cfg,
		Bus:               bus,
		Journal:           store,
		Tools:             reg,
		MCP:               mcpManager,
		provider:          prov,
		runner:            runner,
		shutdownTelemetry: shutdownTelemetry,
	}, nil
}

func (a *App) RunPrompt(ctx context.Context, prompt string) (agent.RunResult, error) {
	ctx, span := tracer.Start(ctx, "godex.run_prompt",
		trace.WithAttributes(
			attribute.String("gode.provider", a.Config.Provider),
			attribute.String("gode.model", a.Config.Model),
		),
	)
	defer span.End()

	result, err := a.runner.Run(ctx, agent.RunRequest{SessionID: uuid.NewString(), Prompt: prompt})
	if err != nil {
		recordSpanError(span, err)
	}
	return result, err
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
	if a.shutdownTelemetry != nil {
		_ = a.shutdownTelemetry(ctx)
	}
	return nil
}

func recordSpanError(span trace.Span, err error) {
	span.RecordError(err)
	span.SetStatus(codes.Error, err.Error())
}

func buildProvider(cfg Config) (provider.Provider, error) {
	switch cfg.Provider {
	case "mock":
		return provider.NewMock("mock response", nil), nil
	case "codex", "openai":
		if usesCodexAuth(cfg) {
			return provider.NewOpenAI(cfg.Model, cfg.Reasoning, codexauth.OpenAIOptions(cfg.DataDir)...), nil
		}
		return provider.NewOpenAI(cfg.Model, cfg.Reasoning), nil
	default:
		return nil, fmt.Errorf("unknown provider %q", cfg.Provider)
	}
}

func usesCodexAuth(cfg Config) bool {
	cfg = cfg.withDefaults()
	if !strings.HasPrefix(cfg.Model, "gpt-") {
		return false
	}
	if cfg.Provider == "codex" {
		return true
	}
	return cfg.Provider == "openai" && (codexauth.Store{DataDir: cfg.DataDir}).SignedIn()
}
