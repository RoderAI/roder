package appserver

import (
	"context"
	"encoding/json"
	"strings"

	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/memory"
)

type settingsUpdateParams struct {
	DefaultModel          *string          `json:"defaultModel,omitempty"`
	DefaultReasoning      *string          `json:"defaultReasoning,omitempty"`
	FastMode              *bool            `json:"fastMode,omitempty"`
	AutoApprove           *bool            `json:"autoApprove,omitempty"`
	MarkdownRendering     *bool            `json:"markdownRendering,omitempty"`
	DisableAutoCompaction *bool            `json:"disableAutoCompaction,omitempty"`
	AutoCompactTokenLimit *int             `json:"autoCompactTokenLimit,omitempty"`
	Memories              *memory.Settings `json:"memories,omitempty"`
}

func (s *Server) handleSettingsGet() any {
	settings, _ := godex.LoadSettings(s.app.Config.DataDir)
	return s.settingsView(settings)
}

func (s *Server) handleSettingsUpdate(ctx context.Context, raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[settingsUpdateParams](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	settings, err := godex.LoadSettings(s.app.Config.DataDir)
	if err != nil {
		return nil, rpcError(errorInternal, err.Error())
	}

	if params.DefaultModel != nil || params.DefaultReasoning != nil {
		model := s.app.Config.Model
		if params.DefaultModel != nil {
			model = strings.TrimSpace(*params.DefaultModel)
		}
		reasoning := s.app.Config.Reasoning
		if params.DefaultReasoning != nil {
			reasoning = strings.TrimSpace(*params.DefaultReasoning)
		}
		if err := s.app.SetModelReasoning(model, reasoning); err != nil {
			return nil, rpcError(errorInvalidParams, err.Error())
		}
		settings.DefaultModel = s.app.Config.Model
		settings.DefaultReasoning = s.app.Config.Reasoning
	}
	if params.FastMode != nil {
		if err := s.app.SetFastMode(*params.FastMode); err != nil {
			return nil, rpcError(errorInvalidParams, err.Error())
		}
		settings.FastMode = *params.FastMode
	}
	if params.AutoApprove != nil {
		s.app.SetAutoApprove(*params.AutoApprove)
		settings.AutoApprove = *params.AutoApprove
	}
	needsRunnerRefresh := false
	if params.MarkdownRendering != nil {
		s.app.Config.MarkdownRendering = *params.MarkdownRendering
		settings.MarkdownRendering = *params.MarkdownRendering
	}
	if params.DisableAutoCompaction != nil {
		s.app.Config.DisableAutoCompaction = *params.DisableAutoCompaction
		settings.DisableAutoCompaction = *params.DisableAutoCompaction
		needsRunnerRefresh = true
	}
	if params.AutoCompactTokenLimit != nil {
		if *params.AutoCompactTokenLimit < 0 {
			return nil, rpcError(errorInvalidParams, "autoCompactTokenLimit must be non-negative")
		}
		s.app.Config.AutoCompactTokenLimit = *params.AutoCompactTokenLimit
		settings.AutoCompactTokenLimit = *params.AutoCompactTokenLimit
		needsRunnerRefresh = true
	}
	if params.Memories != nil {
		settings.Memories = mergeMemorySettings(settings.Memories, *params.Memories)
		s.app.Config.Memories = memory.ApplySettings(s.app.Config.Memories, settings.Memories)
		if err := s.app.SetMemoriesEnabled(s.app.Config.Memories.Enabled); err != nil {
			return nil, rpcError(errorInvalidParams, err.Error())
		}
	}
	if needsRunnerRefresh {
		if err := s.app.SetFastMode(s.app.Config.FastMode); err != nil {
			return nil, rpcError(errorInvalidParams, err.Error())
		}
	}
	if err := godex.SaveSettings(s.app.Config.DataDir, settings); err != nil {
		return nil, rpcError(errorInternal, err.Error())
	}
	s.app.Bus.Publish(ctx, eventbus.Event{
		Kind:    eventbus.KindSettingsChanged,
		Source:  eventbus.SourceSystem,
		Payload: s.settingsView(settings),
	})
	return s.settingsView(settings), nil
}

func (s *Server) settingsView(settings godex.Settings) map[string]any {
	cfg := s.app.Config
	return map[string]any{
		"config": map[string]any{
			"workspace":             cfg.Workspace,
			"dataDir":               cfg.DataDir,
			"provider":              cfg.Provider,
			"model":                 cfg.Model,
			"reasoning":             cfg.Reasoning,
			"fastMode":              cfg.FastMode,
			"autoApprove":           cfg.AutoApprove,
			"markdownRendering":     cfg.MarkdownRendering,
			"disableAutoCompaction": cfg.DisableAutoCompaction,
			"autoCompactTokenLimit": cfg.AutoCompactTokenLimit,
			"memories":              cfg.Memories,
		},
		"settings": settings,
	}
}

func mergeMemorySettings(base memory.Settings, patch memory.Settings) memory.Settings {
	if patch.Enabled != nil {
		base.Enabled = patch.Enabled
	}
	if patch.AutoRecall != nil {
		base.AutoRecall = patch.AutoRecall
	}
	if patch.AutoObserve != nil {
		base.AutoObserve = patch.AutoObserve
	}
	if strings.TrimSpace(patch.EmbeddingModel) != "" {
		base.EmbeddingModel = strings.TrimSpace(patch.EmbeddingModel)
	}
	if patch.RecallLimit > 0 {
		base.RecallLimit = patch.RecallLimit
	}
	if strings.TrimSpace(patch.DatabasePath) != "" {
		base.DatabasePath = strings.TrimSpace(patch.DatabasePath)
	}
	return base
}
