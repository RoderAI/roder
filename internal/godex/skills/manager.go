package skills

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

type ActivationState string

const (
	ActivationEnabled  ActivationState = "enabled"
	ActivationDisabled ActivationState = "disabled"
	ActivationMissing  ActivationState = "missing"
)

type ActivationSettings struct {
	ActiveSkills map[string]bool
	SkillSources map[string]string
}

type Manager struct {
	Workspace    string
	DataDir      string
	HomeDir      string
	Env          []string
	LoadSettings func(context.Context) (ActivationSettings, error)
	SaveSettings func(context.Context, ActivationSettings) error
	RunCommand   CommandRunner
}

type ManagedSkill struct {
	Name        string
	Description string
	Path        string
	Source      string
	State       ActivationState
	Diagnostic  string
}

type RecommendedSkillState struct {
	Name   string
	Source string
	State  ActivationState
}

type InstallRequest struct {
	Source string
	Scope  InstallScope
}

type ManagerInstallResult struct {
	Source    string
	Installed []InstalledSkill
	Stdout    string
	Stderr    string
}

func (m *Manager) List(ctx context.Context) ([]ManagedSkill, error) {
	settings, err := m.load(ctx)
	if err != nil {
		return nil, err
	}
	catalog := Discover(DiscoverOptions{Workspace: m.Workspace, DataDir: m.DataDir, HomeDir: m.HomeDir, Env: m.Env})
	items := make([]ManagedSkill, 0, len(catalog.Skills)+len(catalog.Diagnostics))
	for _, skill := range catalog.Skills {
		state := ActivationEnabled
		if !IsSkillEnabled(settings.ActiveSkills, skill.Name) {
			state = ActivationDisabled
		}
		items = append(items, ManagedSkill{
			Name:        skill.Name,
			Description: skill.Description,
			Path:        skill.Path,
			Source:      settings.SkillSources[skill.Name],
			State:       state,
		})
	}
	for _, diagnostic := range catalog.Diagnostics {
		items = append(items, ManagedSkill{Path: diagnostic.Path, State: ActivationMissing, Diagnostic: diagnostic.Message})
	}
	return items, nil
}

func (m *Manager) SetEnabled(ctx context.Context, name string, enabled bool) error {
	settings, err := m.load(ctx)
	if err != nil {
		return err
	}
	SetSkillEnabled(&settings.ActiveSkills, strings.TrimSpace(name), enabled)
	return m.save(ctx, settings)
}

func (m *Manager) Recommended(ctx context.Context) ([]RecommendedSkillState, error) {
	settings, err := m.load(ctx)
	if err != nil {
		return nil, err
	}
	catalog := Discover(DiscoverOptions{Workspace: m.Workspace, DataDir: m.DataDir, HomeDir: m.HomeDir, Env: m.Env})
	installed := map[string]struct{}{}
	for _, skill := range catalog.Skills {
		installed[skill.Name] = struct{}{}
	}
	out := make([]RecommendedSkillState, 0, len(RecommendedDefaultSkills))
	for _, skill := range RecommendedDefaultSkills {
		state := ActivationMissing
		if _, ok := installed[skill.Name]; ok {
			state = ActivationEnabled
			if !IsSkillEnabled(settings.ActiveSkills, skill.Name) {
				state = ActivationDisabled
			}
		}
		out = append(out, RecommendedSkillState{Name: skill.Name, Source: skill.Source, State: state})
	}
	return out, nil
}

func (m *Manager) Install(ctx context.Context, req InstallRequest) (ManagerInstallResult, error) {
	source := strings.TrimSpace(req.Source)
	if source == "" {
		return ManagerInstallResult{}, fmt.Errorf("source is required")
	}
	scope := req.Scope
	if scope == "" {
		scope = InstallScopeGlobal
	}
	result := ManagerInstallResult{Source: source}
	if isLocalSource(source) {
		installResult, err := Install(ctx, InstallOptions{Source: source, Workspace: m.Workspace, DataDir: m.DataDir, Global: scope == InstallScopeGlobal, Project: scope == InstallScopeProject, Yes: true})
		if err != nil {
			return result, err
		}
		result.Installed = installResult.Installed
		settings, err := m.load(ctx)
		if err != nil {
			return result, err
		}
		for _, installed := range installResult.Installed {
			SetSkillEnabled(&settings.ActiveSkills, installed.Name, true)
			if settings.SkillSources == nil {
				settings.SkillSources = map[string]string{}
			}
			settings.SkillSources[installed.Name] = source
		}
		return result, m.save(ctx, settings)
	}
	runner := m.RunCommand
	if runner == nil {
		runner = defaultCommandRunner
	}
	stdout, stderr, err := runner(ctx, NPXInstallCommand(source, scope, m.Workspace, m.DataDir))
	result.Stdout = stdout
	result.Stderr = stderr
	if err != nil {
		return result, err
	}
	settings, err := m.load(ctx)
	if err != nil {
		return result, err
	}
	name := recommendedNameForSource(source)
	if name != "" {
		result.Installed = append(result.Installed, InstalledSkill{Name: name})
		SetSkillEnabled(&settings.ActiveSkills, name, true)
		if settings.SkillSources == nil {
			settings.SkillSources = map[string]string{}
		}
		settings.SkillSources[name] = source
	}
	return result, m.save(ctx, settings)
}

func (m *Manager) InstallRecommended(ctx context.Context, names []string) ([]ManagerInstallResult, error) {
	wanted := map[string]struct{}{}
	for _, name := range names {
		if strings.TrimSpace(name) != "" {
			wanted[strings.TrimSpace(name)] = struct{}{}
		}
	}
	var results []ManagerInstallResult
	for _, skill := range RecommendedDefaultSkills {
		if len(wanted) > 0 {
			if _, ok := wanted[skill.Name]; !ok {
				continue
			}
		}
		result, err := m.Install(ctx, InstallRequest{Source: skill.Source, Scope: InstallScopeGlobal})
		if err != nil {
			return results, err
		}
		results = append(results, result)
	}
	return results, nil
}

func (m *Manager) load(ctx context.Context) (ActivationSettings, error) {
	if m.LoadSettings == nil {
		return ActivationSettings{}, nil
	}
	return m.LoadSettings(ctx)
}

func (m *Manager) save(ctx context.Context, settings ActivationSettings) error {
	if m.SaveSettings == nil {
		return nil
	}
	return m.SaveSettings(ctx, settings)
}

func isLocalSource(source string) bool {
	if strings.HasPrefix(source, ".") || strings.HasPrefix(source, "/") {
		return true
	}
	if _, err := os.Stat(source); err == nil {
		return true
	}
	if _, err := os.Stat(filepath.Clean(source)); err == nil {
		return true
	}
	return false
}

func recommendedNameForSource(source string) string {
	for _, skill := range RecommendedDefaultSkills {
		if skill.Source == source {
			return skill.Name
		}
	}
	if _, name, ok := strings.Cut(source, "@"); ok {
		return strings.TrimSpace(name)
	}
	return ""
}
