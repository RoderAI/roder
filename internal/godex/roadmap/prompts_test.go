package roadmap

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestRoadmapContextPromptIncludesDocumentTaskDiagnosticsAndSkill(t *testing.T) {
	doc, err := Parse("roadmap/30-test.md", sampleRoadmap("first task"))
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	validation := ValidationResult{Path: doc.Path, Diagnostics: []Diagnostic{{
		Path:    doc.Path,
		Line:    7,
		Message: "missing acceptance criteria",
	}}}

	prompt := RoadmapContextPrompt(doc, &doc.Tasks[0], validation, "Use checkbox evidence.")

	for _, want := range []string{
		"roadmapping mode",
		"Roadmap: Test",
		"Focused task: first task",
		"missing acceptance criteria",
		"Roadmap planning skill:",
		"Use checkbox evidence.",
	} {
		if !strings.Contains(prompt, want) {
			t.Fatalf("prompt missing %q:\n%s", want, prompt)
		}
	}
}

func TestLoadPlanningSkillBodyReadsWorkspaceSkillAndIgnoresMissing(t *testing.T) {
	workspace := t.TempDir()
	body, err := LoadPlanningSkillBody(workspace)
	if err != nil {
		t.Fatalf("missing skill should not error: %v", err)
	}
	if body != "" {
		t.Fatalf("missing skill body = %q", body)
	}

	path := filepath.Join(workspace, planningSkillPath)
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(path, []byte("roadmap skill body"), 0o644); err != nil {
		t.Fatalf("write skill: %v", err)
	}
	body, err = LoadPlanningSkillBody(workspace)
	if err != nil {
		t.Fatalf("load skill: %v", err)
	}
	if body != "roadmap skill body" {
		t.Fatalf("body = %q", body)
	}
}
