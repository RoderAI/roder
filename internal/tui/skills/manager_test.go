package skills

import (
	"testing"

	godeskills "github.com/pandelisz/gode/internal/godex/skills"
)

func TestViewStateMapsInstalledAndRecommendedSkills(t *testing.T) {
	state := ViewState([]godeskills.ManagedSkill{
		{
			Name:        "go-development",
			Description: "Go dev",
			Path:        "/repo/.agents/skills/go-development/SKILL.md",
			Interface:   &godeskills.SkillInterface{DisplayName: "Go Development", ShortDescription: "Go short"},
			Dependencies: &godeskills.SkillDependencies{Tools: []godeskills.SkillToolDependency{{
				Type:  "mcp",
				Value: "filesystem",
			}}},
			State: godeskills.ActivationEnabled,
		},
		{Name: "repo-navigation", Description: "Repo nav", Path: "/Users/pz/.gode/skills/repo-navigation/SKILL.md", State: godeskills.ActivationDisabled},
	}, []godeskills.RecommendedSkillState{
		{Name: "go-development", Source: "pandelisz/gode@go-development", State: godeskills.ActivationEnabled},
	})

	if state.InstalledN != 2 || state.EnabledN != 1 {
		t.Fatalf("counts = %d/%d", state.EnabledN, state.InstalledN)
	}
	if state.Installed[0].Scope != "project" || !state.Installed[0].Enabled {
		t.Fatalf("first installed = %#v", state.Installed[0])
	}
	if state.Installed[0].DisplayName != "Go Development" || state.Installed[0].Description != "Go short" {
		t.Fatalf("first installed metadata = %#v", state.Installed[0])
	}
	if len(state.Installed[0].DependencyHints) != 1 || state.Installed[0].DependencyHints[0] != "mcp:filesystem" {
		t.Fatalf("dependency hints = %#v", state.Installed[0].DependencyHints)
	}
	if state.Installed[1].Scope != "global" || state.Installed[1].Enabled {
		t.Fatalf("second installed = %#v", state.Installed[1])
	}
	if len(state.Recommended) != 1 || state.Recommended[0].Source == "" {
		t.Fatalf("recommended = %#v", state.Recommended)
	}
}

func TestViewStateMarksAmbiguousSkillNames(t *testing.T) {
	state := ViewState([]godeskills.ManagedSkill{
		{Name: "dup", Path: "/repo/a/SKILL.md", State: godeskills.ActivationEnabled},
		{Name: "dup", Path: "/repo/b/SKILL.md", State: godeskills.ActivationEnabled},
	}, nil)
	if !state.Installed[0].AmbiguousName || !state.Installed[1].AmbiguousName {
		t.Fatalf("installed = %#v", state.Installed)
	}
}

func TestMissingRecommendedNames(t *testing.T) {
	state := ViewState(nil, []godeskills.RecommendedSkillState{
		{Name: "one", State: godeskills.ActivationMissing},
		{Name: "two", State: godeskills.ActivationEnabled},
	})
	names := MissingRecommendedNames(state.Recommended)
	if len(names) != 1 || names[0] != "one" {
		t.Fatalf("names = %#v", names)
	}
}
