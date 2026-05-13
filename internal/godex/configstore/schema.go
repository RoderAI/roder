package configstore

func Schema() map[string]any {
	return map[string]any{
		"type": "object",
		"properties": map[string]any{
			"workspace":                map[string]any{"type": "string"},
			"data_dir":                 map[string]any{"type": "string"},
			"provider":                 map[string]any{"type": "string"},
			"model":                    map[string]any{"type": "string"},
			"reasoning":                map[string]any{"type": "string"},
			"default_model":            map[string]any{"type": "string"},
			"default_reasoning":        map[string]any{"type": "string"},
			"fast_mode":                map[string]any{"type": "boolean"},
			"auto_approve":             map[string]any{"type": "boolean"},
			"disable_auto_compaction":  map[string]any{"type": "boolean"},
			"auto_compact_token_limit": map[string]any{"type": "integer"},
			"telemetry":                map[string]any{"type": "boolean"},
			"telemetry_endpoint":       map[string]any{"type": "string"},
			"context_paths":            map[string]any{"type": "array", "items": map[string]any{"type": "string"}},
			"disabled_tools":           map[string]any{"type": "array", "items": map[string]any{"type": "string"}},
			"mcp":                      map[string]any{"type": "object"},
			"provider_config":          map[string]any{"type": "object"},
			"selected_models":          map[string]any{"type": "object"},
		},
	}
}
