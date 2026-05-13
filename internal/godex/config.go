package godex

import (
	"os"
	"path/filepath"
	"runtime"

	"github.com/pandelisz/gode/internal/godex/provider"
)

type Config struct {
	Workspace         string
	DataDir           string
	Provider          string
	Model             string
	Reasoning         string
	FastMode          bool
	AutoApprove       bool
	Telemetry         bool
	TelemetryEndpoint string
	MCP               map[string]any
	ProviderConfig    map[string]provider.ProviderConfig
	SelectedModels    map[string]provider.SelectedModel
	ContextPaths      []string
	DisabledTools     []string
}

func DefaultConfig() Config {
	wd, _ := os.Getwd()
	defaultModel := DefaultModelConfig()
	return Config{
		Workspace:         wd,
		DataDir:           defaultDataDir(),
		Provider:          defaultModel.Provider,
		Model:             defaultModel.ID,
		Reasoning:         defaultModel.DefaultReasoning,
		AutoApprove:       false,
		Telemetry:         false,
		TelemetryEndpoint: "localhost:4317",
		MCP:               map[string]any{},
		ProviderConfig:    map[string]provider.ProviderConfig{},
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
	if c.Model == "" {
		if provider, ok := LookupProvider(c.Provider); ok && provider.DefaultModel != "" {
			c.Model = provider.DefaultModel
		} else {
			c.Model = defaults.Model
		}
	}
	if c.Provider == "" {
		c.Provider = ModelConfigFor(c.Model).Provider
	}
	if c.Reasoning == "" {
		c.Reasoning = ModelConfigFor(c.Model).DefaultReasoning
	}
	if c.TelemetryEndpoint == "" {
		c.TelemetryEndpoint = defaults.TelemetryEndpoint
	}
	if c.MCP == nil {
		c.MCP = defaults.MCP
	}
	if c.ProviderConfig == nil {
		c.ProviderConfig = defaults.ProviderConfig
	}
	if c.SelectedModels == nil {
		c.SelectedModels = defaults.SelectedModels
	}
	return c
}
