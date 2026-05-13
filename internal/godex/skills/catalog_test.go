package skills

import (
	"reflect"
	"testing"
)

func TestSkillActivationDefaultsToEnabled(t *testing.T) {
	if !IsSkillEnabled(nil, "go-tests") {
		t.Fatal("nil active skills should enable discovered skills")
	}
	active := map[string]bool{"go-tests": false}
	if IsSkillEnabled(active, "go-tests") {
		t.Fatal("explicit false should disable skill")
	}
	if !IsSkillEnabled(active, "new-skill") {
		t.Fatal("missing skill should default to enabled")
	}
}

func TestSetSkillEnabledAndEnabledSkillNames(t *testing.T) {
	var active map[string]bool
	SetSkillEnabled(&active, "go-tests", false)
	SetSkillEnabled(&active, "review", true)
	if active["go-tests"] || !active["review"] {
		t.Fatalf("active = %#v", active)
	}
	names := EnabledSkillNames(active, []Skill{{Name: "go-tests"}, {Name: "review"}, {Name: "new-skill"}})
	if !reflect.DeepEqual(names, []string{"new-skill", "review"}) {
		t.Fatalf("names = %#v", names)
	}
}
