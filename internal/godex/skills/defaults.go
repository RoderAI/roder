package skills

type RecommendedSkill struct {
	Name   string
	Source string
}

var RecommendedDefaultSkills = []RecommendedSkill{
	{Name: "go-development", Source: "pandelisz/gode@go-development"},
	{Name: "repo-navigation", Source: "pandelisz/gode@repo-navigation"},
	{Name: "test-driven-go", Source: "pandelisz/gode@test-driven-go"},
	{Name: "terminal-debugging", Source: "pandelisz/gode@terminal-debugging"},
}
