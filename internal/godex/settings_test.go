package godex

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex/memory"
	godeskills "github.com/pandelisz/gode/internal/godex/skills"
)

func TestSettingsRoundTripDefaultModel(t *testing.T) {
	dataDir := t.TempDir()

	if err := SaveSettings(dataDir, Settings{
		DefaultModel:          "gpt-5.5",
		DefaultReasoning:      ReasoningHigh,
		FastMode:              true,
		AutoApprove:           true,
		DisableAutoCompaction: true,
		AutoCompactTokenLimit: 12345,
		Memories: memory.Settings{
			Enabled:        boolPtr(false),
			AutoRecall:     boolPtr(false),
			AutoObserve:    boolPtr(true),
			EmbeddingModel: "custom-embedding",
			RecallLimit:    7,
			DatabasePath:   "custom.sqlite3",
		},
		Skills: godeskills.Config{Rules: []godeskills.ConfigRule{
			{Name: "go-tests", Enabled: true},
			{Name: "disabled-skill", Enabled: false},
		}},
		SkillSources: map[string]string{"go-tests": "pandelisz/gode@go-tests"},
	}); err != nil {
		t.Fatalf("save settings: %v", err)
	}

	settings, err := LoadSettings(dataDir)
	if err != nil {
		t.Fatalf("load settings: %v", err)
	}
	if settings.DefaultModel != "gpt-5.5" {
		t.Fatalf("default model = %q", settings.DefaultModel)
	}
	if settings.DefaultReasoning != ReasoningHigh {
		t.Fatalf("default reasoning = %q", settings.DefaultReasoning)
	}
	if !settings.FastMode {
		t.Fatal("fast mode = false")
	}
	if !settings.AutoApprove {
		t.Fatal("auto approve = false")
	}
	if !settings.DisableAutoCompaction {
		t.Fatal("disable auto compaction = false")
	}
	if settings.AutoCompactTokenLimit != 12345 {
		t.Fatalf("auto compact token limit = %d", settings.AutoCompactTokenLimit)
	}
	if settings.Memories.Enabled == nil || *settings.Memories.Enabled {
		t.Fatalf("memory enabled = %#v", settings.Memories.Enabled)
	}
	if settings.Memories.AutoRecall == nil || *settings.Memories.AutoRecall {
		t.Fatalf("memory auto recall = %#v", settings.Memories.AutoRecall)
	}
	if settings.Memories.AutoObserve == nil || !*settings.Memories.AutoObserve {
		t.Fatalf("memory auto observe = %#v", settings.Memories.AutoObserve)
	}
	if settings.Memories.EmbeddingModel != "custom-embedding" || settings.Memories.RecallLimit != 7 || settings.Memories.DatabasePath != "custom.sqlite3" {
		t.Fatalf("memory settings = %#v", settings.Memories)
	}
	if len(settings.Skills.Rules) != 2 || settings.Skills.Rules[0].Name != "go-tests" || !settings.Skills.Rules[0].Enabled || settings.Skills.Rules[1].Name != "disabled-skill" || settings.Skills.Rules[1].Enabled {
		t.Fatalf("skills config = %#v", settings.Skills)
	}
	if settings.SkillSources["go-tests"] != "pandelisz/gode@go-tests" {
		t.Fatalf("skill sources = %#v", settings.SkillSources)
	}
	if _, err := os.Stat(filepath.Join(dataDir, "config.toml")); err != nil {
		t.Fatalf("config.toml should be written: %v", err)
	}
	data, err := os.ReadFile(filepath.Join(dataDir, "config.toml"))
	if err != nil {
		t.Fatalf("read config.toml: %v", err)
	}
	for _, want := range []string{`default_model = "gpt-5.5"`, `default_reasoning = "high"`, `fast_mode = true`, `auto_approve = true`, `disable_auto_compaction = true`, `auto_compact_token_limit = 12345`, `[memories]`, `enabled = false`, `auto_recall = false`, `auto_observe = true`, `embedding_model = "custom-embedding"`, `recall_limit = 7`, `database_path = "custom.sqlite3"`, `[[skills.config]]`, `name = "go-tests"`, `name = "disabled-skill"`, `enabled = false`, `[skill_sources]`, `go-tests = "pandelisz/gode@go-tests"`} {
		if !strings.Contains(string(data), want) {
			t.Fatalf("config.toml should contain %q, got:\n%s", want, string(data))
		}
	}
	if strings.Contains(string(data), "{") {
		t.Fatalf("config.toml should contain TOML default model, got:\n%s", string(data))
	}

	if _, err := LoadSettings(filepath.Join(dataDir, "missing")); err != nil {
		t.Fatalf("missing settings should load empty defaults: %v", err)
	}
}

func TestDefaultMemoryConfig(t *testing.T) {
	dataDir := filepath.Join(t.TempDir(), "data")
	cfg := Config{DataDir: dataDir}.withDefaults()
	if !cfg.Memories.Enabled {
		t.Fatal("memories should default to enabled")
	}
	if !cfg.Memories.AutoRecall {
		t.Fatal("memory auto recall should default to enabled")
	}
	if cfg.Memories.AutoObserve {
		t.Fatal("memory auto observe should default to disabled")
	}
	if cfg.Memories.EmbeddingModel != memory.DefaultEmbeddingModel {
		t.Fatalf("embedding model = %q", cfg.Memories.EmbeddingModel)
	}
	if cfg.Memories.RecallLimit != 5 {
		t.Fatalf("recall limit = %d", cfg.Memories.RecallLimit)
	}
	if cfg.Memories.DatabasePath != filepath.Join(dataDir, "memories.sqlite3") {
		t.Fatalf("database path = %q", cfg.Memories.DatabasePath)
	}
}

func TestDefaultDataDirUsesWindowsConfigDir(t *testing.T) {
	if got := defaultDataDirFor("windows", "C:/Users/pz", "C:/Users/pz/AppData/Roaming"); got != filepath.Join("C:/Users/pz/AppData/Roaming", "gode") {
		t.Fatalf("windows data dir = %q", got)
	}
	if got := defaultDataDirFor("linux", "/home/pz", "/tmp/config"); got != filepath.Join("/home/pz", ".gode") {
		t.Fatalf("linux data dir = %q", got)
	}
	if got := defaultDataDirFor("darwin", "/Users/pz", "/tmp/config"); got != filepath.Join("/Users/pz", ".gode") {
		t.Fatalf("darwin data dir = %q", got)
	}
}

func boolPtr(value bool) *bool {
	return &value
}
