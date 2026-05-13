package memory

import (
	"path/filepath"
	"strings"
	"time"
)

const (
	DefaultEmbeddingModel = "text-embedding-3-large"
	DefaultRecallLimit    = 5
	MaxRecallLimit        = 20
)

type Config struct {
	Enabled        bool
	AutoRecall     bool
	AutoObserve    bool
	EmbeddingModel string
	RecallLimit    int
	DatabasePath   string
}

type Settings struct {
	Enabled        *bool  `json:"enabled,omitempty" toml:"enabled,omitempty"`
	AutoRecall     *bool  `json:"auto_recall,omitempty" toml:"auto_recall,omitempty"`
	AutoObserve    *bool  `json:"auto_observe,omitempty" toml:"auto_observe,omitempty"`
	EmbeddingModel string `json:"embedding_model,omitempty" toml:"embedding_model,omitempty"`
	RecallLimit    int    `json:"recall_limit,omitempty" toml:"recall_limit,omitempty"`
	DatabasePath   string `json:"database_path,omitempty" toml:"database_path,omitempty"`
}

type Entry struct {
	ID             string
	WorkspaceID    string
	WorkspaceRoot  string
	Content        string
	ContentHash    string
	Source         string
	CreatedAt      time.Time
	UpdatedAt      time.Time
	DeletedAt      *time.Time
	EmbeddingModel string
	EmbeddingDims  int
	Score          float64
	Metadata       map[string]string
}

type Vector struct {
	Model      string
	Dimensions int
	Values     []float32
}

func DefaultConfig(dataDir string) Config {
	return Config{
		Enabled:        true,
		AutoRecall:     true,
		AutoObserve:    false,
		EmbeddingModel: DefaultEmbeddingModel,
		RecallLimit:    DefaultRecallLimit,
		DatabasePath:   defaultDatabasePath(dataDir),
	}
}

func ApplySettings(cfg Config, settings Settings) Config {
	if settings.Enabled != nil {
		cfg.Enabled = *settings.Enabled
	}
	if settings.AutoRecall != nil {
		cfg.AutoRecall = *settings.AutoRecall
	}
	if settings.AutoObserve != nil {
		cfg.AutoObserve = *settings.AutoObserve
	}
	if model := strings.TrimSpace(settings.EmbeddingModel); model != "" {
		cfg.EmbeddingModel = model
	}
	if settings.RecallLimit > 0 {
		cfg.RecallLimit = settings.RecallLimit
	}
	if path := strings.TrimSpace(settings.DatabasePath); path != "" {
		cfg.DatabasePath = path
	}
	return cfg.withDefaults("")
}

func (cfg Config) WithDefaults(dataDir string) Config {
	if cfg.IsZero() {
		return DefaultConfig(dataDir)
	}
	return cfg.withDefaults(dataDir)
}

func (cfg Config) IsZero() bool {
	return !cfg.Enabled &&
		!cfg.AutoRecall &&
		!cfg.AutoObserve &&
		cfg.EmbeddingModel == "" &&
		cfg.RecallLimit == 0 &&
		strings.TrimSpace(cfg.DatabasePath) == ""
}

func (cfg Config) withDefaults(dataDir string) Config {
	defaults := DefaultConfig(dataDir)
	if cfg.EmbeddingModel == "" {
		cfg.EmbeddingModel = defaults.EmbeddingModel
	}
	if cfg.RecallLimit <= 0 {
		cfg.RecallLimit = defaults.RecallLimit
	}
	if cfg.RecallLimit > MaxRecallLimit {
		cfg.RecallLimit = MaxRecallLimit
	}
	if strings.TrimSpace(cfg.DatabasePath) == "" {
		cfg.DatabasePath = defaults.DatabasePath
	}
	return cfg
}

func defaultDatabasePath(dataDir string) string {
	if strings.TrimSpace(dataDir) == "" {
		return "memories.sqlite3"
	}
	return filepath.Join(dataDir, "memories.sqlite3")
}
