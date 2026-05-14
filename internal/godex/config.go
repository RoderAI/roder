package godex

import (
	"os"
	"path/filepath"
	"runtime"

	"github.com/pandelisz/gode/internal/godex/lsp"
	"github.com/pandelisz/gode/internal/godex/mcp"
	"github.com/pandelisz/gode/internal/godex/memory"
	"github.com/pandelisz/gode/internal/godex/provider"
	godeskills "github.com/pandelisz/gode/internal/godex/skills"
)

type Config struct {
	Workspace             string
	DataDir               string
	Provider              string
	Model                 string
	Reasoning             string
	FastMode              bool
	AutoApprove           bool
	TimelineStyle         string
	MarkdownRendering     bool
	Memories              memory.Config
	Skills                godeskills.Config
	DisableAutoCompaction bool
	AutoCompactTokenLimit int
	GoalsEnabled          bool
	DisableGoals          bool
	Telemetry             bool
	TelemetryEndpoint     string
	MCP                   map[string]mcp.ServerConfig
	LSP                   map[string]lsp.Config
	ProviderConfig        map[string]provider.ProviderConfig
	UserModels            map[string]provider.UserModelConfig
	SelectedModels        map[string]provider.SelectedModel
	ContextPaths          []string
	DisabledTools         []string
}

func DefaultConfig() Config {
	defaultModel := DefaultModelConfig()
	return Config{
		Workspace:         ".",
		DataDir:           defaultDataDir(),
		Provider:          defaultModel.Provider,
		Model:             defaultModel.ID,
		Reasoning:         defaultModel.DefaultReasoning,
		AutoApprove:       false,
		TimelineStyle:     TimelineStyleMinimal,
		MarkdownRendering: true,
		Memories:          memory.DefaultConfig(defaultDataDir()),
		GoalsEnabled:      true,
		Telemetry:         false,
		TelemetryEndpoint: "localhost:4317",
		MCP:               map[string]mcp.ServerConfig{},
		LSP:               map[string]lsp.Config{},
		ProviderConfig:    map[string]provider.ProviderConfig{},
		UserModels:        map[string]provider.UserModelConfig{},
		SelectedModels:    map[string]provider.SelectedModel{},
	}
}

func defaultDataDir() string {
	home, _ := os.UserHomeDir()
	userConfigDir := ""
	if runtime.GOOS == "windows" {
		userConfigDir, _ = os.UserConfigDir()
	}
	return defaultDataDirFor(runtime.GOOS, home, userConfigDir)
}

func defaultDataDirFor(goos string, home string, userConfigDir string) string {
	if goos == "windows" && userConfigDir != "" {
		return filepath.Join(userConfigDir, "gode")
	}
	return filepath.Join(home, ".gode")
}

func (c Config) withDefaults() Config {
	defaults := DefaultConfig()
	if c.Workspace == "" {
		c.Workspace = defaults.Workspace
	}
	if c.DataDir == "" {
		c.DataDir = defaults.DataDir
	}
	if c.Memories.DatabasePath == defaults.Memories.DatabasePath {
		c.Memories.DatabasePath = ""
	}
	c.Memories = c.Memories.WithDefaults(c.DataDir)
	if c.Model == "" {
		if provider, ok := LookupProviderForConfig(c, c.Provider); ok && provider.DefaultModel != "" {
			c.Model = provider.DefaultModel
		} else {
			c.Model = defaults.Model
		}
	}
	modelConfig := ModelConfigForConfig(c, c.Model)
	if c.Provider == "" || c.Provider == defaults.Provider {
		c.Provider = modelConfig.Provider
	}
	if c.Reasoning == "" {
		c.Reasoning = modelConfig.DefaultReasoning
	}
	c.TimelineStyle = NormalizeTimelineStyle(c.TimelineStyle)
	if c.TelemetryEndpoint == "" {
		c.TelemetryEndpoint = defaults.TelemetryEndpoint
	}
	if !c.DisableGoals {
		c.GoalsEnabled = true
	}
	if c.MCP == nil {
		c.MCP = defaults.MCP
	}
	if c.LSP == nil {
		c.LSP = defaults.LSP
	}
	if c.ProviderConfig == nil {
		c.ProviderConfig = defaults.ProviderConfig
	}
	if c.UserModels == nil {
		c.UserModels = defaults.UserModels
	}
	if c.SelectedModels == nil {
		c.SelectedModels = defaults.SelectedModels
	}
	return c
}
