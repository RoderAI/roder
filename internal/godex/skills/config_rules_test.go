package skills

import (
	"path/filepath"
	"reflect"
	"testing"
)

func TestSkillConfigRulesDefaultToEnabled(t *testing.T) {
	skill := Skill{Name: "go-tests", Path: filepath.Join(t.TempDir(), "go-tests", "SKILL.md")}
	if !IsSkillEnabled(Config{}, skill) {
		t.Fatal("skills should be enabled by default")
	}
	cfg := Config{Rules: []ConfigRule{{Name: "go-tests", Enabled: false}}}
	if IsSkillEnabled(cfg, skill) {
		t.Fatal("name rule should disable skill")
	}
}

func TestSkillConfigPathRuleOverridesNameRule(t *testing.T) {
	dir := t.TempDir()
	first := Skill{Name: "dup", Path: filepath.Join(dir, "one", "SKILL.md")}
	second := Skill{Name: "dup", Path: filepath.Join(dir, "two", "SKILL.md")}
	cfg := Config{Rules: []ConfigRule{
		{Name: "dup", Enabled: false},
		{Path: second.Path, Enabled: true},
	}}
	names := EnabledSkillNames(cfg, []Skill{first, second})
	if !reflect.DeepEqual(names, []string{"dup"}) {
		t.Fatalf("enabled names = %#v", names)
	}
	if IsSkillEnabled(cfg, first) {
		t.Fatal("first duplicate should remain disabled")
	}
	if !IsSkillEnabled(cfg, second) {
		t.Fatal("path override should enable second duplicate")
	}
}

func TestSetSkillEnabledWritesPathRule(t *testing.T) {
	skill := Skill{Name: "go-tests", Path: filepath.Join(t.TempDir(), "go-tests", "SKILL.md")}
	cfg := Config{Rules: []ConfigRule{{Name: "go-tests", Enabled: true}}}
	SetSkillEnabled(&cfg, skill, false)
	if len(cfg.Rules) != 2 || cfg.Rules[1].Path == "" || cfg.Rules[1].Enabled {
		t.Fatalf("rules = %#v", cfg.Rules)
	}
}
