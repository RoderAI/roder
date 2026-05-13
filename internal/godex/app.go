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
	godecommands "github.com/pandelisz/gode/internal/godex/commands"
	"github.com/pandelisz/gode/internal/godex/contextpack"
	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/hooks"
	"github.com/pandelisz/gode/internal/godex/journal"
	"github.com/pandelisz/gode/internal/godex/lsp"
	"github.com/pandelisz/gode/internal/godex/mcp"
	messagestore "github.com/pandelisz/gode/internal/godex/message"
	"github.com/pandelisz/gode/internal/godex/permission"
	"github.com/pandelisz/gode/internal/godex/provider"
	"github.com/pandelisz/gode/internal/godex/repoconfig"
	"github.com/pandelisz/gode/internal/godex/session"
	godeskills "github.com/pandelisz/gode/internal/godex/skills"
	godetelemetry "github.com/pandelisz/gode/internal/godex/telemetry"
	"github.com/pandelisz/gode/internal/godex/tools"
	"github.com/pandelisz/gode/internal/godex/tools/builtin"
	"go.opentelemetry.io/otel"
	"go.opentelemetry.io/otel/attribute"
	"go.opentelemetry.io/otel/codes"
	"go.opentelemetry.io/otel/trace"
)

type App struct {
	Config   Config
	Bus      *eventbus.Bus
	Journal  *journal.Store
	Sessions *session.Store
	Messages *messagestore.Store
	Tools    *tools.Registry
	MCP      *mcp.Manager
	LSP      *lsp.Manager

	provider          provider.Provider
	runner            *agent.Runner
	contextMessages   []provider.Message
	skills            []godeskills.Skill
	commands          []godecommands.Command
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
	sessionStore, err := session.Open(cfg.DataDir)
	if err != nil {
		_ = store.Close()
		recordSpanError(span, err)
		_ = shutdownTelemetry(ctx)
		return nil, err
	}
	messageStore := messagestore.Open(cfg.DataDir)
	repoContext, err := loadContextMessages(cfg.Workspace)
	if err != nil {
		_ = store.Close()
		recordSpanError(span, err)
		_ = shutdownTelemetry(ctx)
		return nil, err
	}
	skillCatalog := godeskills.Discover(godeskills.DiscoverOptions{Workspace: cfg.Workspace, DataDir: cfg.DataDir})
	commandCatalog, err := godecommands.Load(godecommands.LoadOptions{Workspace: cfg.Workspace})
	if err != nil {
		_ = store.Close()
		recordSpanError(span, err)
		_ = shutdownTelemetry(ctx)
		return nil, err
	}

	permissionService := permission.New(permission.WithEventBus(bus))
	hookRunner := hooks.New(nil)
	reg := tools.NewRegistry(
		tools.WithEventBus(bus),
		tools.WithAutoApprove(cfg.AutoApprove),
		tools.WithPermissionService(permissionService),
		tools.WithHookRunner(hookRunner),
		tools.WithWorkspace(cfg.Workspace),
	)
	builtin.RegisterFilesystem(reg, cfg.Workspace)
	builtin.RegisterSearch(reg, cfg.Workspace)
	builtin.RegisterEditing(reg, cfg.Workspace)
	builtin.RegisterDownload(reg, cfg.Workspace)
	builtin.RegisterGit(reg, cfg.Workspace)
	builtin.RegisterTodo(reg)
	builtin.RegisterMemory(reg, filepath.Join(cfg.DataDir, "memory.jsonl"))
	builtin.RegisterShell(reg, cfg.Workspace)
	builtin.RegisterPatch(reg, cfg.Workspace)
	builtin.RegisterSubagent(reg)

	mcpManager := mcp.NewManager(bus, cfg.MCP)
	_ = mcpManager.Start(ctx)
	builtin.RegisterMCP(reg, mcpManager)
	lspManager := lsp.NewManager(bus, cfg.Workspace, cfg.LSP)
	_ = lspManager.Start(ctx)
	builtin.RegisterLSP(reg, lspManager)

	prov, err := buildProvider(cfg)
	if err != nil {
		store.Close()
		recordSpanError(span, err)
		_ = shutdownTelemetry(ctx)
		return nil, err
	}
	runner := agent.NewRunner(agent.Config{Bus: bus, Journal: store, Sessions: sessionStore, Messages: messageStore, Tools: reg, Provider: prov, ContextMessages: repoContext, Skills: skillCatalog.Skills, Commands: commandCatalog.Commands})

	return &App{
		Config:            cfg,
		Bus:               bus,
		Journal:           store,
		Sessions:          sessionStore,
		Messages:          messageStore,
		Tools:             reg,
		MCP:               mcpManager,
		LSP:               lspManager,
		provider:          prov,
		runner:            runner,
		contextMessages:   repoContext,
		skills:            skillCatalog.Skills,
		commands:          commandCatalog.Commands,
		shutdownTelemetry: shutdownTelemetry,
	}, nil
}

func (a *App) RunPrompt(ctx context.Context, prompt string) (agent.RunResult, error) {
	return a.runner.Run(ctx, agent.RunRequest{SessionID: uuid.NewString(), Prompt: prompt})
}

func (a *App) Commands() []godecommands.Command {
	return append([]godecommands.Command(nil), a.commands...)
}

func (a *App) SetModel(model string) error {
	return a.SetModelReasoning(model, "")
}

func (a *App) SetModelReasoning(model string, reasoning string) error {
	model = strings.TrimSpace(model)
	if model == "" {
		return fmt.Errorf("model is required")
	}

	cfg := a.Config
	cfg.Model = model
	modelConfig := ModelConfigFor(model)
	reasoning = strings.TrimSpace(reasoning)
	if reasoning == "" {
		reasoning = modelConfig.DefaultReasoning
	}
	if !modelConfig.SupportsReasoning(reasoning) {
		return fmt.Errorf("model %q does not support reasoning %q", model, reasoning)
	}
	cfg.Provider = modelConfig.Provider
	cfg.Reasoning = reasoning
	prov, err := buildProvider(cfg)
	if err != nil {
		return err
	}

	a.Config = cfg
	a.provider = prov
	a.runner = agent.NewRunner(agent.Config{Bus: a.Bus, Journal: a.Journal, Sessions: a.Sessions, Messages: a.Messages, Tools: a.Tools, Provider: prov, ContextMessages: a.contextMessages, Skills: a.skills, Commands: a.commands})
	return nil
}

func (a *App) SetFastMode(fastMode bool) error {
	cfg := a.Config
	cfg.FastMode = fastMode
	prov, err := buildProvider(cfg)
	if err != nil {
		return err
	}

	a.Config = cfg
	a.provider = prov
	a.runner = agent.NewRunner(agent.Config{Bus: a.Bus, Journal: a.Journal, Sessions: a.Sessions, Messages: a.Messages, Tools: a.Tools, Provider: prov, ContextMessages: a.contextMessages, Skills: a.skills, Commands: a.commands})
	return nil
}

func (a *App) Close(ctx context.Context) error {
	if a.MCP != nil {
		_ = a.MCP.Close(ctx)
	}
	if a.LSP != nil {
		_ = a.LSP.Close(ctx)
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
	providerConfig, ok := LookupProvider(cfg.Provider)
	if !ok {
		return nil, fmt.Errorf("unknown provider %q", cfg.Provider)
	}
	switch providerConfig.Kind {
	case ProviderKindMock:
		return provider.NewMock("mock response", nil), nil
	case ProviderKindOpenAI:
		openAIConfig := provider.OpenAIConfig{
			Model:       cfg.Model,
			Reasoning:   cfg.Reasoning,
			ServiceTier: openAIServiceTier(cfg),
		}
		if UsesCodexAuth(cfg) {
			return provider.NewOpenAIWithConfig(openAIConfig, codexauth.OpenAIOptions(cfg.DataDir)...), nil
		}
		return provider.NewOpenAIWithConfig(openAIConfig), nil
	default:
		return nil, fmt.Errorf("unknown provider kind %q for %q", providerConfig.Kind, cfg.Provider)
	}
}

func loadContextMessages(workspace string) ([]provider.Message, error) {
	repo, err := repoconfig.Load(workspace)
	if err != nil {
		return nil, err
	}
	pack, err := contextpack.Load(contextpack.LoadOptions{Workspace: workspace, Repo: repo})
	if err != nil {
		return nil, err
	}
	return pack.Messages(), nil
}

func openAIServiceTier(cfg Config) string {
	if cfg.FastMode {
		return "priority"
	}
	return ""
}

func DisplayProvider(cfg Config) string {
	cfg = cfg.withDefaults()
	if UsesCodexAuth(cfg) {
		return ProviderCodex
	}
	return cfg.Provider
}

func DisplayModelLabel(cfg Config) string {
	cfg = cfg.withDefaults()
	provider := DisplayProvider(cfg)
	if provider == "" {
		return cfg.Model
	}
	if cfg.Model == "" {
		return provider
	}
	return provider + "/" + cfg.Model
}

func UsesCodexAuth(cfg Config) bool {
	cfg = cfg.withDefaults()
	if !strings.HasPrefix(cfg.Model, "gpt-") {
		return false
	}
	if cfg.Provider == ProviderCodex {
		return true
	}
	return cfg.Provider == ProviderOpenAI && (codexauth.Store{DataDir: cfg.DataDir}).SignedIn()
}
