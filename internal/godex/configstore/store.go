package configstore

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"

	"github.com/BurntSushi/toml"
	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/lsp"
	"github.com/pandelisz/gode/internal/godex/mcp"
	"github.com/pandelisz/gode/internal/godex/provider"
)

type Source string

const (
	SourceDefaults Source = "defaults"
	SourceGlobal   Source = "global"
	SourceProject  Source = "project"
	SourceData     Source = "data"
	SourceEnv      Source = "env"
	SourceFlags    Source = "flags"
)

type Loaded struct {
	Config godex.Config
	Paths  []string
}

type LoadOptions struct {
	Workspace string
	DataDir   string
	Env       []string
	Flags     godex.Config
	FlagSet   map[string]bool
}

type overlay struct {
	Workspace             *string                            `json:"workspace,omitempty" toml:"workspace,omitempty"`
	DataDir               *string                            `json:"data_dir,omitempty" toml:"data_dir,omitempty"`
	Provider              *string                            `json:"provider,omitempty" toml:"provider,omitempty"`
	Model                 *string                            `json:"model,omitempty" toml:"model,omitempty"`
	Reasoning             *string                            `json:"reasoning,omitempty" toml:"reasoning,omitempty"`
	DefaultModel          *string                            `json:"default_model,omitempty" toml:"default_model,omitempty"`
	DefaultReasoning      *string                            `json:"default_reasoning,omitempty" toml:"default_reasoning,omitempty"`
	FastMode              *bool                              `json:"fast_mode,omitempty" toml:"fast_mode,omitempty"`
	AutoApprove           *bool                              `json:"auto_approve,omitempty" toml:"auto_approve,omitempty"`
	TimelineStyle         *string                            `json:"timeline_style,omitempty" toml:"timeline_style,omitempty"`
	DisableAutoCompaction *bool                              `json:"disable_auto_compaction,omitempty" toml:"disable_auto_compaction,omitempty"`
	AutoCompactTokenLimit *int                               `json:"auto_compact_token_limit,omitempty" toml:"auto_compact_token_limit,omitempty"`
	Telemetry             *bool                              `json:"telemetry,omitempty" toml:"telemetry,omitempty"`
	TelemetryEndpoint     *string                            `json:"telemetry_endpoint,omitempty" toml:"telemetry_endpoint,omitempty"`
	MCP                   map[string]any                     `json:"mcp,omitempty" toml:"mcp,omitempty"`
	LSP                   map[string]lsp.Config              `json:"lsp,omitempty" toml:"lsp,omitempty"`
	ProviderConfig        map[string]provider.ProviderConfig `json:"provider_config,omitempty" toml:"provider_config,omitempty"`
	SelectedModels        map[string]provider.SelectedModel  `json:"selected_models,omitempty" toml:"selected_models,omitempty"`
	ContextPaths          []string                           `json:"context_paths,omitempty" toml:"context_paths,omitempty"`
	DisabledTools         []string                           `json:"disabled_tools,omitempty" toml:"disabled_tools,omitempty"`
}

func Load(opts LoadOptions) (Loaded, error) {
	env := envMap(opts.Env)
	cfg := godex.DefaultConfig()
	if opts.Workspace != "" {
		cfg.Workspace = opts.Workspace
	}
	if opts.DataDir != "" {
		cfg.DataDir = opts.DataDir
	}

	var paths []string
	if path := globalConfigPath(env); path != "" {
		if loaded, err := applyFile(&cfg, SourceGlobal, path); err != nil {
			return Loaded{}, err
		} else if loaded {
			paths = append(paths, path)
		}
	}
	if path := projectConfigPath(cfg.Workspace); path != "" {
		if loaded, err := applyFile(&cfg, SourceProject, path); err != nil {
			return Loaded{}, err
		} else if loaded {
			paths = append(paths, path)
		}
	}
	if isSet(opts.FlagSet, "data-dir") || isSet(opts.FlagSet, "data_dir") {
		cfg.DataDir = opts.Flags.DataDir
	}
	for _, path := range dataConfigPaths(cfg.DataDir) {
		if loaded, err := applyFile(&cfg, SourceData, path); err != nil {
			return Loaded{}, err
		} else if loaded {
			paths = append(paths, path)
			break
		}
	}
	if err := applyEnv(&cfg, env); err != nil {
		return Loaded{}, err
	}
	applyFlags(&cfg, opts.Flags, opts.FlagSet)
	fillDerivedDefaults(&cfg)
	return Loaded{Config: cfg, Paths: paths}, nil
}

func applyFile(cfg *godex.Config, source Source, path string) (bool, error) {
	data, err := os.ReadFile(path)
	if os.IsNotExist(err) {
		return false, nil
	}
	if err != nil {
		return false, fmt.Errorf("read %s config %s: %w", source, path, err)
	}
	if strings.TrimSpace(string(data)) == "" {
		return true, nil
	}
	var patch overlay
	switch strings.ToLower(filepath.Ext(path)) {
	case ".json":
		if err := json.Unmarshal(data, &patch); err != nil {
			return false, fmt.Errorf("parse %s config %s: %w", source, path, err)
		}
	case ".toml":
		if err := toml.Unmarshal(data, &patch); err != nil {
			return false, fmt.Errorf("parse %s config %s: %w", source, path, err)
		}
	default:
		return false, fmt.Errorf("parse %s config %s: unsupported extension", source, path)
	}
	if err := applyOverlay(cfg, patch); err != nil {
		return false, fmt.Errorf("parse %s config %s: %w", source, path, err)
	}
	return true, nil
}

func applyOverlay(cfg *godex.Config, patch overlay) error {
	if patch.Workspace != nil {
		cfg.Workspace = strings.TrimSpace(*patch.Workspace)
	}
	if patch.DataDir != nil {
		cfg.DataDir = strings.TrimSpace(*patch.DataDir)
	}
	if patch.Provider != nil {
		cfg.Provider = strings.TrimSpace(*patch.Provider)
	}
	if patch.Model != nil {
		cfg.Model = strings.TrimSpace(*patch.Model)
	}
	if patch.DefaultModel != nil {
		cfg.Model = strings.TrimSpace(*patch.DefaultModel)
	}
	if patch.Reasoning != nil {
		cfg.Reasoning = strings.TrimSpace(*patch.Reasoning)
	}
	if patch.DefaultReasoning != nil {
		cfg.Reasoning = strings.TrimSpace(*patch.DefaultReasoning)
	}
	if patch.FastMode != nil {
		cfg.FastMode = *patch.FastMode
	}
	if patch.AutoApprove != nil {
		cfg.AutoApprove = *patch.AutoApprove
	}
	if patch.TimelineStyle != nil {
		cfg.TimelineStyle = godex.NormalizeTimelineStyle(*patch.TimelineStyle)
	}
	if patch.DisableAutoCompaction != nil {
		cfg.DisableAutoCompaction = *patch.DisableAutoCompaction
	}
	if patch.AutoCompactTokenLimit != nil {
		cfg.AutoCompactTokenLimit = *patch.AutoCompactTokenLimit
	}
	if patch.Telemetry != nil {
		cfg.Telemetry = *patch.Telemetry
	}
	if patch.TelemetryEndpoint != nil {
		cfg.TelemetryEndpoint = strings.TrimSpace(*patch.TelemetryEndpoint)
	}
	if patch.MCP != nil {
		parsed, err := mcp.ParseConfigMap(patch.MCP)
		if err != nil {
			return err
		}
		cfg.MCP = mergeMap(cfg.MCP, parsed)
	}
	if patch.LSP != nil {
		cfg.LSP = mergeMap(cfg.LSP, patch.LSP)
	}
	if patch.ProviderConfig != nil {
		cfg.ProviderConfig = mergeMap(cfg.ProviderConfig, patch.ProviderConfig)
	}
	if patch.SelectedModels != nil {
		cfg.SelectedModels = mergeMap(cfg.SelectedModels, patch.SelectedModels)
	}
	if patch.ContextPaths != nil {
		cfg.ContextPaths = append([]string(nil), patch.ContextPaths...)
	}
	if patch.DisabledTools != nil {
		cfg.DisabledTools = append([]string(nil), patch.DisabledTools...)
	}
	return nil
}

func applyEnv(cfg *godex.Config, env map[string]string) error {
	if value := strings.TrimSpace(env["GODE_PROVIDER"]); value != "" {
		cfg.Provider = value
	}
	if value := strings.TrimSpace(env["GODE_MODEL"]); value != "" {
		cfg.Model = value
	}
	if value := strings.TrimSpace(env["GODE_REASONING"]); value != "" {
		cfg.Reasoning = value
	}
	if value := strings.TrimSpace(env["GODE_AUTO_APPROVE"]); value != "" {
		parsed, err := strconv.ParseBool(value)
		if err != nil {
			return fmt.Errorf("parse env GODE_AUTO_APPROVE: %w", err)
		}
		cfg.AutoApprove = parsed
	}
	if value := strings.TrimSpace(env["GODE_TIMELINE_STYLE"]); value != "" {
		cfg.TimelineStyle = godex.NormalizeTimelineStyle(value)
	}
	if value := strings.TrimSpace(env["GODE_DISABLE_AUTO_COMPACTION"]); value != "" {
		parsed, err := strconv.ParseBool(value)
		if err != nil {
			return fmt.Errorf("parse env GODE_DISABLE_AUTO_COMPACTION: %w", err)
		}
		cfg.DisableAutoCompaction = parsed
	}
	if value := strings.TrimSpace(env["GODE_AUTO_COMPACT_TOKEN_LIMIT"]); value != "" {
		parsed, err := strconv.Atoi(value)
		if err != nil {
			return fmt.Errorf("parse env GODE_AUTO_COMPACT_TOKEN_LIMIT: %w", err)
		}
		cfg.AutoCompactTokenLimit = parsed
	}
	return nil
}

func applyFlags(cfg *godex.Config, flags godex.Config, set map[string]bool) {
	if isSet(set, "workspace") {
		cfg.Workspace = flags.Workspace
	}
	if isSet(set, "data-dir") || isSet(set, "data_dir") {
		cfg.DataDir = flags.DataDir
	}
	if isSet(set, "provider") {
		cfg.Provider = flags.Provider
	}
	if isSet(set, "model") {
		cfg.Model = flags.Model
	}
	if isSet(set, "reasoning") {
		cfg.Reasoning = flags.Reasoning
	}
	if isSet(set, "fast-mode") || isSet(set, "fast_mode") {
		cfg.FastMode = flags.FastMode
	}
	if isSet(set, "auto-approve") || isSet(set, "auto_approve") {
		cfg.AutoApprove = flags.AutoApprove
	}
	if isSet(set, "timeline-style") || isSet(set, "timeline_style") {
		cfg.TimelineStyle = godex.NormalizeTimelineStyle(flags.TimelineStyle)
	}
	if isSet(set, "disable-auto-compaction") || isSet(set, "disable_auto_compaction") {
		cfg.DisableAutoCompaction = flags.DisableAutoCompaction
	}
	if isSet(set, "auto-compact-token-limit") || isSet(set, "auto_compact_token_limit") {
		cfg.AutoCompactTokenLimit = flags.AutoCompactTokenLimit
	}
	if isSet(set, "telemetry") {
		cfg.Telemetry = flags.Telemetry
	}
	if isSet(set, "telemetry-endpoint") || isSet(set, "telemetry_endpoint") {
		cfg.TelemetryEndpoint = flags.TelemetryEndpoint
	}
}

func fillDerivedDefaults(cfg *godex.Config) {
	defaults := godex.DefaultConfig()
	if cfg.Workspace == "" {
		cfg.Workspace = defaults.Workspace
	}
	if cfg.DataDir == "" {
		cfg.DataDir = defaults.DataDir
	}
	if cfg.Provider == "" {
		cfg.Provider = godex.ModelConfigFor(cfg.Model).Provider
	}
	if cfg.Model == "" {
		if providerConfig, ok := godex.LookupProvider(cfg.Provider); ok && providerConfig.DefaultModel != "" {
			cfg.Model = providerConfig.DefaultModel
		} else {
			cfg.Model = defaults.Model
		}
	}
	if cfg.Reasoning == "" {
		cfg.Reasoning = godex.ModelConfigFor(cfg.Model).DefaultReasoning
	}
	cfg.TimelineStyle = godex.NormalizeTimelineStyle(cfg.TimelineStyle)
	if cfg.TelemetryEndpoint == "" {
		cfg.TelemetryEndpoint = defaults.TelemetryEndpoint
	}
	if cfg.MCP == nil {
		cfg.MCP = map[string]mcp.ServerConfig{}
	}
	if cfg.LSP == nil {
		cfg.LSP = map[string]lsp.Config{}
	}
	if cfg.ProviderConfig == nil {
		cfg.ProviderConfig = map[string]provider.ProviderConfig{}
	}
	if cfg.SelectedModels == nil {
		cfg.SelectedModels = map[string]provider.SelectedModel{}
	}
}

func globalConfigPath(env map[string]string) string {
	if xdg := strings.TrimSpace(env["XDG_CONFIG_HOME"]); xdg != "" {
		return filepath.Join(xdg, "gode", "config.json")
	}
	home := strings.TrimSpace(env["HOME"])
	if home == "" && runtime.GOOS == "windows" {
		home = strings.TrimSpace(env["USERPROFILE"])
	}
	if home == "" {
		return ""
	}
	return filepath.Join(home, ".config", "gode", "config.json")
}

func projectConfigPath(workspace string) string {
	dir := strings.TrimSpace(workspace)
	if dir == "" {
		dir = godex.DefaultConfig().Workspace
	}
	abs, err := filepath.Abs(dir)
	if err == nil {
		dir = abs
	}
	if info, err := os.Stat(dir); err == nil && !info.IsDir() {
		dir = filepath.Dir(dir)
	}
	for {
		for _, name := range []string{".gode.json", "gode.json", ".gode.toml", "gode.toml"} {
			path := filepath.Join(dir, name)
			if _, err := os.Stat(path); err == nil {
				return path
			}
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			return ""
		}
		dir = parent
	}
}

func dataConfigPaths(dataDir string) []string {
	if dataDir == "" {
		dataDir = godex.DefaultConfig().DataDir
	}
	return []string{
		filepath.Join(dataDir, "config.toml"),
		filepath.Join(dataDir, "settings.json"),
	}
}

func envMap(env []string) map[string]string {
	if env == nil {
		env = os.Environ()
	}
	out := map[string]string{}
	for _, entry := range env {
		key, value, ok := strings.Cut(entry, "=")
		if !ok {
			continue
		}
		out[key] = value
	}
	return out
}

func mergeMap[T any](base map[string]T, patch map[string]T) map[string]T {
	out := make(map[string]T, len(base)+len(patch))
	for key, value := range base {
		out[key] = value
	}
	for key, value := range patch {
		out[key] = value
	}
	return out
}

func isSet(set map[string]bool, key string) bool {
	return set != nil && set[key]
}
