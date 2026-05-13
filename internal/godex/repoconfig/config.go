package repoconfig

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/BurntSushi/toml"
)

var configNames = []string{".gode.toml", "gode.toml"}

type Config struct {
	Path            string
	Dir             string
	IsTopmostConfig bool        `toml:"is_topmost_config"`
	Agent           AgentConfig `toml:"agent"`
	Hooks           HookConfig  `toml:"hooks"`
}

type AgentConfig struct {
	Model        string   `toml:"model"`
	ExtraContext string   `toml:"extra_context"`
	ContextPaths []string `toml:"context_paths"`
}

type HookConfig struct {
	OnSave []string `toml:"on_save"`
}

type Loaded struct {
	Configs []Config
}

func Load(workspace string) (Loaded, error) {
	dir, err := workspaceDir(workspace)
	if err != nil {
		return Loaded{}, err
	}

	var leafFirst [][]Config
	for {
		configs, stop, err := loadDir(dir)
		if err != nil {
			return Loaded{}, err
		}
		if len(configs) > 0 {
			leafFirst = append(leafFirst, configs)
		}
		if stop {
			break
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			break
		}
		dir = parent
	}

	var configs []Config
	for i := len(leafFirst) - 1; i >= 0; i-- {
		configs = append(configs, leafFirst[i]...)
	}
	return Loaded{Configs: configs}, nil
}

func (l Loaded) Model() string {
	for i := len(l.Configs) - 1; i >= 0; i-- {
		if model := strings.TrimSpace(l.Configs[i].Agent.Model); model != "" {
			return model
		}
	}
	return ""
}

func workspaceDir(workspace string) (string, error) {
	if strings.TrimSpace(workspace) == "" {
		workspace = "."
	}
	abs, err := filepath.Abs(workspace)
	if err != nil {
		return "", fmt.Errorf("workspace: %w", err)
	}
	info, err := os.Stat(abs)
	if err == nil && !info.IsDir() {
		abs = filepath.Dir(abs)
	}
	if err != nil && !os.IsNotExist(err) {
		return "", fmt.Errorf("stat workspace %s: %w", abs, err)
	}
	return abs, nil
}

func loadDir(dir string) ([]Config, bool, error) {
	var configs []Config
	stop := false
	for _, name := range configNames {
		path := filepath.Join(dir, name)
		info, err := os.Stat(path)
		if os.IsNotExist(err) {
			continue
		}
		if err != nil {
			return nil, false, fmt.Errorf("stat repo config %s: %w", path, err)
		}
		if info.IsDir() {
			continue
		}
		cfg, err := read(path)
		if err != nil {
			return nil, false, err
		}
		configs = append(configs, cfg)
		if cfg.IsTopmostConfig {
			stop = true
		}
	}
	return configs, stop, nil
}

func read(path string) (Config, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return Config{}, fmt.Errorf("read repo config %s: %w", path, err)
	}
	cfg := Config{Path: path, Dir: filepath.Dir(path)}
	if strings.TrimSpace(string(data)) == "" {
		return cfg, nil
	}
	if err := toml.Unmarshal(data, &cfg); err != nil {
		return Config{}, fmt.Errorf("parse repo config %s: %w", path, err)
	}
	cfg.Path = path
	cfg.Dir = filepath.Dir(path)
	return cfg, nil
}
