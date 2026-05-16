package skills

import (
	"testing"

	godeskills "github.com/pandelisz/gode/internal/godex/skills"
)

func TestViewStateMapsInstalledAndRecommendedSkills(t *testing.T) {
	state := ViewState([]godeskills.ManagedSkill{
		{Name: "go-development", Description: "Go dev", Path: "/repo/.agents/skills/go-development/SKILL.md", State: godeskills.ActivationEnabled},
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
	if state.Installed[1].Scope != "global" || state.Installed[1].Enabled {
		t.Fatalf("second installed = %#v", state.Installed[1])
	}
	if len(state.Recommended) != 1 || state.Recommended[0].Source == "" {
		t.Fatalf("recommended = %#v", state.Recommended)
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
