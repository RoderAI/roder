package skills

import (
	"path/filepath"
	"strings"
	"testing"
)

func TestRenderAvailableSkillsUsesAliasesAndWarnings(t *testing.T) {
	root := filepath.Join(t.TempDir(), "skills")
	catalog := Catalog{
		Roots: []Root{{Path: root, Scope: SkillScopeRepo}},
	}
	for _, name := range []string{"one", "two", "three", "four", "five", "six"} {
		catalog.Skills = append(catalog.Skills, Skill{Name: name, Description: strings.Repeat(name, 120), Path: filepath.Join(root, name, "SKILL.md")})
	}
	rendered := RenderAvailable(catalog, RenderOptions{ContextWindow: 10000})
	if !strings.Contains(rendered.Body, "### Skill roots") || !strings.Contains(rendered.Body, "`r0`") || !strings.Contains(rendered.Body, "r0/one/SKILL.md") {
		t.Fatalf("body =\n%s", rendered.Body)
	}
	if rendered.Report.TotalCount != 6 || rendered.Report.IncludedCount == 0 {
		t.Fatalf("report = %#v", rendered.Report)
	}
	if rendered.Report.Warning == "" {
		t.Fatalf("expected budget warning, report = %#v", rendered.Report)
	}
}

func TestRenderAvailableSkillsExcludesDisabled(t *testing.T) {
	path := filepath.Join(t.TempDir(), "one", "SKILL.md")
	rendered := RenderAvailable(Catalog{Skills: []Skill{{Name: "one", Description: "desc", Path: path}}}, RenderOptions{
		Config: Config{Rules: []ConfigRule{{Path: path, Enabled: false}}},
	})
	if strings.TrimSpace(rendered.Body) != "" || rendered.Report.TotalCount != 0 {
		t.Fatalf("rendered = %#v", rendered)
	}
}
