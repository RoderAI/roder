package appserver

import "github.com/pandelisz/gode/internal/godex"

func (s *Server) handleModelList() any {
	models := godex.ModelsForConfig(s.app.Config, false)
	out := make([]map[string]any, 0, len(models))
	for _, model := range models {
		out = append(out, map[string]any{
			"id":                     model.ID,
			"name":                   model.DisplayName,
			"description":            model.Description,
			"modelProvider":          model.Provider,
			"reasoningEfforts":       model.ReasoningEfforts(),
			"defaultReasoningEffort": model.DefaultReasoning,
			"contextWindow":          model.ContextWindow,
			"maxContextWindow":       model.MaxContextWindow,
			"isDefault":              model.ID == godex.DefaultModelID,
		})
	}
	return map[string]any{
		"models": out,
	}
}
