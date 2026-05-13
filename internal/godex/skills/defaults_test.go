package skills

import "testing"

func TestRecommendedDefaultsAreDeterministic(t *testing.T) {
	if len(RecommendedDefaultSkills) != 4 {
		t.Fatalf("recommended = %#v", RecommendedDefaultSkills)
	}
	if RecommendedDefaultSkills[0].Name != "go-development" || RecommendedDefaultSkills[0].Source != "pandelisz/gode@go-development" {
		t.Fatalf("first recommended = %#v", RecommendedDefaultSkills[0])
	}
}
