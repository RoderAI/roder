package godex

import (
	"os"
	"path/filepath"
)

type Config struct {
	Workspace   string
	DataDir     string
	Provider    string
	Model       string
	AutoApprove bool
	MCP         map[string]any
}

func DefaultConfig() Config {
	wd, _ := os.Getwd()
	home, _ := os.UserHomeDir()
	return Config{
		Workspace:   wd,
		DataDir:     filepath.Join(home, ".gode"),
		Provider:    "mock",
		Model:       "gpt-5.1-codex",
		AutoApprove: false,
		MCP:         map[string]any{},
	}
}

func (c Config) withDefaults() Config {
	defaults := DefaultConfig()
	if c.Workspace == "" {
		c.Workspace = defaults.Workspace
	}
	if c.DataDir == "" {
		c.DataDir = defaults.DataDir
	}
	if c.Provider == "" {
		c.Provider = defaults.Provider
	}
	if c.Model == "" {
		c.Model = defaults.Model
	}
	if c.MCP == nil {
		c.MCP = defaults.MCP
	}
	return c
}
