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
	"github.com/pandelisz/gode/internal/godex/goals"
	"github.com/pandelisz/gode/internal/godex/hooks"
	"github.com/pandelisz/gode/internal/godex/journal"
	"github.com/pandelisz/gode/internal/godex/lsp"
	"github.com/pandelisz/gode/internal/godex/mcp"
	"github.com/pandelisz/gode/internal/godex/memory"
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
	Config       Config
	Bus          *eventbus.Bus
	Journal      *journal.Store
	Sessions     *session.Store
	Turns        *session.TurnStore
	Items        *session.ItemStore
	Messages     *messagestore.Store
	Tools        *tools.Registry
	Goals        *goals.Runtime
	Memory       *memory.Service
	SkillManager *godeskills.Manager
	MCP          *mcp.Manager
	LSP          *lsp.Manager

	provider          provider.Provider
	runner            *agent.Runner
	contextMessages   []provider.Message
	skills            []godeskills.Skill
	commands          []godecommands.Command
	shutdownTelemetry func(context.Context) error
}

type CompactSessionResult struct {
	SessionID   string
	RunID       string
	ResponseID  string
	OutputItems int
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
	turnStore, err := session.OpenTurnStore(cfg.DataDir)
	if err != nil {
		_ = store.Close()
		recordSpanError(span, err)
		_ = shutdownTelemetry(ctx)
		return nil, err
	}
	itemStore, err := session.OpenItemStore(cfg.DataDir)
	if err != nil {
		_ = store.Close()
		recordSpanError(span, err)
		_ = shutdownTelemetry(ctx)
		return nil, err
	}
	goalStore, err := goals.Open(cfg.DataDir)
	if err != nil {
		_ = store.Close()
		recordSpanError(span, err)
		_ = shutdownTelemetry(ctx)
		return nil, err
	}
	goalRuntime := goals.NewRuntime(goalStore, bus, store)
	repoContext, err := loadContextMessages(cfg.Workspace)
	if err != nil {
		_ = store.Close()
		recordSpanError(span, err)
		_ = shutdownTelemetry(ctx)
		return nil, err
	}
	skillCatalog := godeskills.Discover(godeskills.DiscoverOptions{Workspace: cfg.Workspace, DataDir: cfg.DataDir})
	skillManager := newSkillManager(cfg)
	commandCatalog, err := godecommands.Load(godecommands.LoadOptions{Workspace: cfg.Workspace})
	if err != nil {
		_ = store.Close()
		recordSpanError(span, err)
		_ = shutdownTelemetry(ctx)
		return nil, err
	}

	memoryService, err := newMemoryService(ctx, cfg, bus)
	if err != nil {
		_ = store.Close()
		recordSpanError(span, err)
		_ = shutdownTelemetry(ctx)
		return nil, err
	}

	mcpManager := mcp.NewManager(bus, cfg.MCP)
	_ = mcpManager.Start(ctx)
	lspManager := lsp.NewManager(bus, cfg.Workspace, cfg.LSP)
	_ = lspManager.Start(ctx)
	reg := buildToolRegistry(cfg, bus, goalRuntime, memoryService, mcpManager, lspManager)

	prov, err := buildProvider(cfg)
	if err != nil {
		store.Close()
		_ = memoryService.Close()
		recordSpanError(span, err)
		_ = shutdownTelemetry(ctx)
		return nil, err
	}
	runner := agent.NewRunner(runnerConfig(cfg, bus, store, sessionStore, turnStore, itemStore, messageStore, reg, goalRuntime, memoryService, prov, repoContext, skillCatalog.Skills, commandCatalog.Commands))

	return &App{
		Config:            cfg,
		Bus:               bus,
		Journal:           store,
		Sessions:          sessionStore,
		Turns:             turnStore,
		Items:             itemStore,
		Messages:          messageStore,
		Tools:             reg,
		Goals:             goalRuntime,
		Memory:            memoryService,
		SkillManager:      skillManager,
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

func (a *App) Skills() []godeskills.Skill {
	return append([]godeskills.Skill(nil), a.skills...)
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
	a.runner = agent.NewRunner(runnerConfig(cfg, a.Bus, a.Journal, a.Sessions, a.Turns, a.Items, a.Messages, a.Tools, a.Goals, a.Memory, prov, a.contextMessages, a.skills, a.commands))
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
	a.runner = agent.NewRunner(runnerConfig(cfg, a.Bus, a.Journal, a.Sessions, a.Turns, a.Items, a.Messages, a.Tools, a.Goals, a.Memory, prov, a.contextMessages, a.skills, a.commands))
	return nil
}

func (a *App) SetAutoApprove(autoApprove bool) {
	a.Config.AutoApprove = autoApprove
	if a.Tools != nil {
		a.Tools.SetAutoApprove(autoApprove)
	}
}

func (a *App) SetMemoriesEnabled(enabled bool) error {
	cfg := a.Config
	cfg.Memories.Enabled = enabled
	cfg = cfg.withDefaults()
	memoryService, err := newMemoryService(context.Background(), cfg, a.Bus)
	if err != nil {
		return err
	}
	reg := buildToolRegistry(cfg, a.Bus, a.Goals, memoryService, a.MCP, a.LSP)
	if a.Memory != nil {
		_ = a.Memory.Close()
	}
	a.Config = cfg
	a.Memory = memoryService
	a.Tools = reg
	a.runner = agent.NewRunner(runnerConfig(cfg, a.Bus, a.Journal, a.Sessions, a.Turns, a.Items, a.Messages, a.Tools, a.Goals, memoryService, a.provider, a.contextMessages, a.skills, a.commands))
	return nil
}

func newMemoryService(ctx context.Context, cfg Config, bus *eventbus.Bus) (*memory.Service, error) {
	scope, err := memory.NewScope(cfg.Workspace, cfg.Memories.DatabasePath, cfg.DataDir)
	if err != nil {
		return nil, err
	}
	memCfg := cfg.Memories.WithDefaults(cfg.DataDir)
	if !memCfg.Enabled {
		return memory.NewService(nil, nil, scope, memCfg, bus), nil
	}
	store, err := memory.OpenStore(ctx, scope.DatabasePath)
	if err != nil {
		return nil, err
	}
	return memory.NewService(store, memory.NewOpenAIEmbedder(memCfg.EmbeddingModel), scope, memCfg, bus), nil
}

func buildToolRegistry(cfg Config, bus *eventbus.Bus, goalRuntime *goals.Runtime, memoryService *memory.Service, mcpManager *mcp.Manager, lspManager *lsp.Manager) *tools.Registry {
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
	if memoryService != nil && cfg.Memories.Enabled {
		memory.RegisterTools(reg, memoryService)
	}
	builtin.RegisterShell(reg, cfg.Workspace)
	builtin.RegisterPatch(reg, cfg.Workspace)
	builtin.RegisterSubagent(reg)
	if cfg.GoalsEnabled {
		builtin.RegisterGoal(reg, goalRuntime)
	}
	if mcpManager != nil {
		builtin.RegisterMCP(reg, mcpManager)
	}
	if lspManager != nil {
		builtin.RegisterLSP(reg, lspManager)
	}
	return reg
}

func runnerConfig(cfg Config, bus *eventbus.Bus, journalStore *journal.Store, sessionStore *session.Store, turnStore *session.TurnStore, itemStore *session.ItemStore, messageStore *messagestore.Store, registry *tools.Registry, goalRuntime *goals.Runtime, memoryService *memory.Service, prov provider.Provider, contextMessages []provider.Message, skills []godeskills.Skill, commands []godecommands.Command) agent.Config {
	return agent.Config{
		Bus:                   bus,
		Journal:               journalStore,
		Sessions:              sessionStore,
		Turns:                 turnStore,
		Items:                 itemStore,
		Messages:              messageStore,
		Tools:                 registry,
		Provider:              prov,
		Model:                 cfg.Model,
		Workspace:             cfg.Workspace,
		DisableAutoCompaction: cfg.DisableAutoCompaction,
		AutoCompactTokenLimit: cfg.AutoCompactTokenLimit,
		Goals:                 goalRuntime,
		Memory:                memoryService,
		ContextMessages:       contextMessages,
		Skills:                skills,
		LoadActiveSkills:      loadActiveSkills(cfg.DataDir),
		Commands:              commands,
	}
}

func (a *App) Close(ctx context.Context) error {
	if a.MCP != nil {
		_ = a.MCP.Close(ctx)
	}
	if a.LSP != nil {
		_ = a.LSP.Close(ctx)
	}
	if a.Memory != nil {
		_ = a.Memory.Close()
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

func (a *App) publish(ctx context.Context, ev eventbus.Event) eventbus.Event {
	if a.Bus == nil {
		return ev
	}
	return a.Bus.Publish(ctx, ev)
}

func (a *App) appendJournal(ctx context.Context, ev eventbus.Event) {
	if a.Journal != nil {
		_ = a.Journal.Append(ctx, ev)
	}
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
	case ProviderKindAnthropic:
		return provider.NewAnthropicWithConfig(provider.AnthropicConfig{
			Model:   cfg.Model,
			BaseURL: providerConfig.BaseURL,
			APIKey:  os.Getenv(providerConfig.EnvKey),
		}), nil
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
