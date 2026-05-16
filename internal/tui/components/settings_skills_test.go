package components

import (
	"strings"
	"testing"

	zone "github.com/lrstanley/bubblezone/v2"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func TestSkillSettingsContentShowsMetadataDependenciesAndAmbiguity(t *testing.T) {
	resetSettingsStyles()
	lines := skillSettingsContent(120, []viewmodel.SettingsSkillItem{{
		Name:            "go-tests",
		DisplayName:     "Go Tests",
		Description:     "Run the focused Go test suite",
		Path:            "/repo/.agents/skills/go-tests/SKILL.md",
		Scope:           "repo",
		State:           "enabled",
		DependencyHints: []string{"mcp:filesystem"},
		AmbiguousName:   true,
		Selected:        true,
	}}, zone.New())
	view := strings.Join(lines, "\n")
	for _, want := range []string{"Go Tests", "enabled repo", "Run the focused Go test suite", "ambiguous name; path required:", "requires mcp:filesystem"} {
		if !strings.Contains(view, want) {
			t.Fatalf("settings content missing %q:\n%s", want, view)
		}
	}
}
