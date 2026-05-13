package godex

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/BurntSushi/toml"
)

const (
	settingsFileName       = "config.toml"
	legacySettingsJSONName = "settings.json"
)

type Settings struct {
	DefaultModel          string            `json:"default_model,omitempty" toml:"default_model,omitempty"`
	DefaultReasoning      string            `json:"default_reasoning,omitempty" toml:"default_reasoning,omitempty"`
	FastMode              bool              `json:"fast_mode,omitempty" toml:"fast_mode,omitempty"`
	DisableAutoCompaction bool              `json:"disable_auto_compaction,omitempty" toml:"disable_auto_compaction,omitempty"`
	AutoCompactTokenLimit int               `json:"auto_compact_token_limit,omitempty" toml:"auto_compact_token_limit,omitempty"`
	ActiveSkills          map[string]bool   `json:"active_skills,omitempty" toml:"active_skills,omitempty"`
	SkillSources          map[string]string `json:"skill_sources,omitempty" toml:"skill_sources,omitempty"`
}

func LoadSettings(dataDir string) (Settings, error) {
	if dataDir == "" {
		dataDir = DefaultConfig().DataDir
	}
	data, err := os.ReadFile(settingsPath(dataDir))
	if errors.Is(err, os.ErrNotExist) {
		return loadLegacySettings(dataDir)
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
	return settings, nil
}

func SaveSettings(dataDir string, settings Settings) error {
	if dataDir == "" {
		dataDir = DefaultConfig().DataDir
	}
	settings.DefaultModel = strings.TrimSpace(settings.DefaultModel)
	settings.DefaultReasoning = strings.TrimSpace(settings.DefaultReasoning)
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

func legacySettingsPath(dataDir string) string {
	return filepath.Join(dataDir, legacySettingsJSONName)
}

func loadLegacySettings(dataDir string) (Settings, error) {
	data, err := os.ReadFile(legacySettingsPath(dataDir))
	if errors.Is(err, os.ErrNotExist) {
		return Settings{}, nil
	}
	if err != nil {
		return Settings{}, fmt.Errorf("read legacy settings: %w", err)
	}
	var settings struct {
		DefaultModel          string            `json:"default_model"`
		DefaultReasoning      string            `json:"default_reasoning"`
		FastMode              bool              `json:"fast_mode"`
		DisableAutoCompaction bool              `json:"disable_auto_compaction"`
		AutoCompactTokenLimit int               `json:"auto_compact_token_limit"`
		ActiveSkills          map[string]bool   `json:"active_skills"`
		SkillSources          map[string]string `json:"skill_sources"`
	}
	if err := json.Unmarshal(data, &settings); err != nil {
		return Settings{}, fmt.Errorf("parse legacy settings: %w", err)
	}
	return Settings{
		DefaultModel:          strings.TrimSpace(settings.DefaultModel),
		DefaultReasoning:      strings.TrimSpace(settings.DefaultReasoning),
		FastMode:              settings.FastMode,
		DisableAutoCompaction: settings.DisableAutoCompaction,
		AutoCompactTokenLimit: settings.AutoCompactTokenLimit,
		ActiveSkills:          settings.ActiveSkills,
		SkillSources:          settings.SkillSources,
	}, nil
}
