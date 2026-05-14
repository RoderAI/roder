package appserver

import (
	"context"
	"encoding/json"
	"os"
	"strings"

	godeskills "github.com/pandelisz/gode/internal/godex/skills"
)

func (s *Server) handleSkillsList(raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[struct {
		CWD string `json:"cwd"`
	}](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	workspace := s.app.Config.Workspace
	if strings.TrimSpace(params.CWD) != "" {
		workspace = params.CWD
	}
	catalog := godeskills.Discover(godeskills.DiscoverOptions{Workspace: workspace, DataDir: s.app.Config.DataDir})
	settings, err := s.app.SkillManager.LoadSettings(context.Background())
	if err != nil {
		return nil, rpcError(errorInternal, err.Error())
	}
	disabled := godeskills.DisabledSkillPaths(catalog.Skills, settings.Skills)
	skills := make([]map[string]any, 0, len(catalog.Skills))
	for _, skill := range catalog.Skills {
		_, isDisabled := disabled[godeskills.SkillIdentity(skill)]
		skills = append(skills, map[string]any{
			"name":             skill.Name,
			"description":      skill.Description,
			"shortDescription": skill.ShortDescription,
			"path":             skill.Path,
			"scope":            skill.Scope,
			"pluginId":         skill.PluginID,
			"enabled":          !isDisabled,
			"interface":        skill.Interface,
			"dependencies":     skill.Dependencies,
		})
	}
	diagnostics := make([]map[string]any, 0, len(catalog.Diagnostics))
	for _, diagnostic := range catalog.Diagnostics {
		diagnostics = append(diagnostics, map[string]any{"path": diagnostic.Path, "message": diagnostic.Message})
	}
	return map[string]any{"skills": skills, "diagnostics": diagnostics}, nil
}

func (s *Server) handleSkillRead(raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[struct {
		Path string `json:"path"`
	}](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	if strings.TrimSpace(params.Path) == "" {
		return nil, rpcError(errorInvalidParams, "path is required")
	}
	data, err := os.ReadFile(params.Path)
	if err != nil {
		return nil, rpcError(errorInternal, err.Error())
	}
	return map[string]any{"path": params.Path, "content": string(data)}, nil
}

func (s *Server) handleSkillSetEnabled(ctx context.Context, raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[struct {
		Selector string `json:"selector"`
		Path     string `json:"path"`
		Name     string `json:"name"`
		Enabled  bool   `json:"enabled"`
	}](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	selector := firstNonEmpty(params.Selector, params.Path, params.Name)
	if selector == "" {
		return nil, rpcError(errorInvalidParams, "selector, path, or name is required")
	}
	if err := s.app.SkillManager.SetEnabled(ctx, selector, params.Enabled); err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	s.notifyAll(ctx, "skills/changed", map[string]any{"selector": selector, "enabled": params.Enabled})
	return map[string]any{}, nil
}

func firstNonEmpty(values ...string) string {
	for _, value := range values {
		if strings.TrimSpace(value) != "" {
			return strings.TrimSpace(value)
		}
	}
	return ""
}
