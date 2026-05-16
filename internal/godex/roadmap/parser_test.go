package roadmap

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestParseExistingRoadmapFile(t *testing.T) {
	doc, err := ParseFile(filepath.Join("..", "..", "..", "roadmap", "20-roadmapping-mode.md"))
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	if doc.Title == "" || doc.Goal == "" || doc.Architecture == "" || doc.TechStack == "" {
		t.Fatalf("missing sections: %#v", doc)
	}
	if len(doc.Tasks) == 0 || len(doc.RunBlocks) == 0 || len(doc.AcceptanceSections) == 0 {
		t.Fatalf("missing parsed content tasks=%d run=%d acceptance=%d", len(doc.Tasks), len(doc.RunBlocks), len(doc.AcceptanceSections))
	}
	if doc.Tasks[0].ID == "" || doc.Tasks[0].Line == 0 {
		t.Fatalf("task shape = %#v", doc.Tasks[0])
	}
}

func TestTaskIDStableAfterUnrelatedLineEdits(t *testing.T) {
	base := sampleRoadmap("first task")
	doc, err := Parse("roadmap/30-test.md", base)
	if err != nil {
		t.Fatalf("parse base: %v", err)
	}
	edited, err := Parse("roadmap/30-test.md", strings.Replace(base, "# Test\n", "# Test\n\nintro\n", 1))
	if err != nil {
		t.Fatalf("parse edited: %v", err)
	}
	if doc.Tasks[0].ID != edited.Tasks[0].ID {
		t.Fatalf("task id changed: %s != %s", doc.Tasks[0].ID, edited.Tasks[0].ID)
	}
}

func TestSetTaskCheckedPreservesOtherBytes(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "roadmap", "30-test.md")
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	raw := sampleRoadmap("first task")
	if err := os.WriteFile(path, []byte(raw), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}
	doc, err := ParseFile(path)
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	if err := SetTaskChecked(path, doc.Tasks[0].ID, true, "tested"); err != nil {
		t.Fatalf("set task: %v", err)
	}
	updated, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read updated: %v", err)
	}
	want := strings.Replace(raw, "- [ ] first task", "- [x] first task", 1)
	if string(updated) != want {
		t.Fatalf("updated bytes mismatch\n got:\n%s\nwant:\n%s", updated, want)
	}
}

func sampleRoadmap(task string) string {
	return "# Test\n\n**Goal:** Ship it.\n\n**Architecture:** Parse lines.\n\n**Tech Stack:** Go\n\n## Owned Paths\n\n- Create: `x`\n\n## Tasks\n\n- [ ] " + task + "\n\nRun:\n\n```sh\ngo test ./...\n```\n\nAcceptance:\n- Works.\n"
}
