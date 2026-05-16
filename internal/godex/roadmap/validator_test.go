package roadmap

import (
	"strings"
	"testing"
)

func TestValidateExistingRoadmapPasses(t *testing.T) {
	doc, err := ParseFile("../../../roadmap/20-roadmapping-mode.md")
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	if diagnostics := Validate(doc); len(diagnostics) != 0 {
		t.Fatalf("diagnostics = %#v", diagnostics)
	}
}

func TestValidateReportsMissingSectionsAndInvalidPath(t *testing.T) {
	doc, err := Parse("not-roadmap.md", "# Missing\n\n- [ ] task\n")
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	diagnostics := Validate(doc)
	joined := diagnosticMessages(diagnostics)
	for _, want := range []string{"roadmap path", "missing goal", "missing architecture", "missing owned paths", "missing tasks section", "missing run command", "missing acceptance criteria"} {
		if !strings.Contains(joined, want) {
			t.Fatalf("diagnostics missing %q: %#v", want, diagnostics)
		}
	}
}

func TestValidateReportsDuplicateTaskIDs(t *testing.T) {
	raw := sampleRoadmap("same task")
	raw = strings.Replace(raw, "- [ ] same task", "- [ ] same task\n- [ ] same task", 1)
	doc, err := Parse("roadmap/30-test.md", raw)
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	diagnostics := Validate(doc)
	if !strings.Contains(diagnosticMessages(diagnostics), "duplicate task id") {
		t.Fatalf("diagnostics = %#v", diagnostics)
	}
}

func diagnosticMessages(diagnostics []Diagnostic) string {
	var messages []string
	for _, diagnostic := range diagnostics {
		messages = append(messages, diagnostic.Message)
	}
	return strings.Join(messages, "\n")
}
