package godex

import (
	"bytes"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/BurntSushi/toml"
	"github.com/pandelisz/gode/internal/godex/memory"
	godeskills "github.com/pandelisz/gode/internal/godex/skills"
)

const settingsFileName = "config.toml"

type Settings struct {
	DefaultModel          string            `json:"default_model,omitempty" toml:"default_model,omitempty"`
	DefaultReasoning      string            `json:"default_reasoning,omitempty" toml:"default_reasoning,omitempty"`
	FastMode              bool              `json:"fast_mode,omitempty" toml:"fast_mode,omitempty"`
	AutoApprove           bool              `json:"auto_approve" toml:"auto_approve"`
	TimelineStyle         string            `json:"timeline_style,omitempty" toml:"timeline_style,omitempty"`
	MarkdownRendering     bool              `json:"markdown_rendering,omitempty" toml:"markdown_rendering,omitempty"`
	Memories              memory.Settings   `json:"memories,omitempty" toml:"memories,omitempty"`
	DisableAutoCompaction bool              `json:"disable_auto_compaction,omitempty" toml:"disable_auto_compaction,omitempty"`
	AutoCompactTokenLimit int               `json:"auto_compact_token_limit,omitempty" toml:"auto_compact_token_limit,omitempty"`
	Skills                godeskills.Config `json:"skills,omitempty" toml:"skills,omitempty"`
	SkillSources          map[string]string `json:"skill_sources,omitempty" toml:"skill_sources,omitempty"`
}

func LoadSettings(dataDir string) (Settings, error) {
	if dataDir == "" {
		dataDir = DefaultConfig().DataDir
	}
	data, err := os.ReadFile(settingsPath(dataDir))
	if errors.Is(err, os.ErrNotExist) {
		return Settings{}, nil
	}
	if err != nil {
		return Settings{}, fmt.Errorf("read settings: %w", err)
	}
	if strings.TrimSpace(string(data)) == "" {
		return Settings{}, nil
	}

	var settings Settings
	if err := toml.Unmarshal(data, &settings); err != nil {
		return Settings{}, fmt.Errorf("parse settings: %w", err)
	}
	settings.DefaultModel = strings.TrimSpace(settings.DefaultModel)
	settings.DefaultReasoning = strings.TrimSpace(settings.DefaultReasoning)
	settings.TimelineStyle = NormalizeTimelineStyle(settings.TimelineStyle)
	settings.Memories.EmbeddingModel = strings.TrimSpace(settings.Memories.EmbeddingModel)
	settings.Memories.DatabasePath = strings.TrimSpace(settings.Memories.DatabasePath)
	return settings, nil
}

func SaveSettings(dataDir string, settings Settings) error {
	if dataDir == "" {
		dataDir = DefaultConfig().DataDir
	}
	settings.DefaultModel = strings.TrimSpace(settings.DefaultModel)
	settings.DefaultReasoning = strings.TrimSpace(settings.DefaultReasoning)
	settings.TimelineStyle = NormalizeTimelineStyle(settings.TimelineStyle)
	settings.Memories.EmbeddingModel = strings.TrimSpace(settings.Memories.EmbeddingModel)
	settings.Memories.DatabasePath = strings.TrimSpace(settings.Memories.DatabasePath)
	if err := os.MkdirAll(dataDir, 0o700); err != nil {
		return fmt.Errorf("settings dir: %w", err)
	}
	var data bytes.Buffer
	if err := toml.NewEncoder(&data).Encode(settings); err != nil {
		return fmt.Errorf("encode settings: %w", err)
	}
	if err := os.WriteFile(settingsPath(dataDir), data.Bytes(), 0o600); err != nil {
		return fmt.Errorf("write settings: %w", err)
	}
	return nil
}

func settingsPath(dataDir string) string {
	return filepath.Join(dataDir, settingsFileName)
}
