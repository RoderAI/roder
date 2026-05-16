package roadmap

import (
	"os"
	"path/filepath"
	"reflect"
	"testing"
)

func TestListDocumentsSortsAndExcludesIndex(t *testing.T) {
	workspace := t.TempDir()
	for _, name := range []string{"20-mode.md", "00-feature-inventory-and-sequencing.md", "03-tools.md", "alpha.md"} {
		writeFile(t, filepath.Join(workspace, "roadmap", name), "# "+name+"\n")
	}
	docs, err := ListDocumentPaths(workspace, false)
	if err != nil {
		t.Fatalf("list: %v", err)
	}
	got := basenames(docs)
	want := []string{"03-tools.md", "20-mode.md", "alpha.md"}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("docs = %#v, want %#v", got, want)
	}
	withIndex, err := ListDocumentPaths(workspace, true)
	if err != nil {
		t.Fatalf("list with index: %v", err)
	}
	if basenames(withIndex)[0] != "00-feature-inventory-and-sequencing.md" {
		t.Fatalf("with index = %#v", basenames(withIndex))
	}
	summaries, err := ListDocuments(workspace, false)
	if err != nil {
		t.Fatalf("summaries: %v", err)
	}
	if summaries[0].Path != "roadmap/03-tools.md" {
		t.Fatalf("summary path = %q", summaries[0].Path)
	}
}

func TestStateSaveLoadRoundTrip(t *testing.T) {
	dataDir := t.TempDir()
	state := State{
		FocusedDocument: "roadmap/20-roadmapping-mode.md",
		FocusedTaskID:   "task-123",
		AttachedThreads: []ThreadAttachment{{Path: "roadmap/20-roadmapping-mode.md", TaskID: "task-123", ThreadID: "thread-1"}},
	}
	if err := SaveState(dataDir, state); err != nil {
		t.Fatalf("save: %v", err)
	}
	loaded, err := LoadState(dataDir)
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	if !reflect.DeepEqual(loaded, state) {
		t.Fatalf("state = %#v, want %#v", loaded, state)
	}
	if _, err := os.Stat(filepath.Join(dataDir, "roadmaps", "state.json.tmp")); !os.IsNotExist(err) {
		t.Fatalf("temp state file left behind: %v", err)
	}
}

func writeFile(t *testing.T, path string, data string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatalf("mkdir %s: %v", path, err)
	}
	if err := os.WriteFile(path, []byte(data), 0o644); err != nil {
		t.Fatalf("write %s: %v", path, err)
	}
}

func basenames(paths []string) []string {
	out := make([]string, 0, len(paths))
	for _, path := range paths {
		out = append(out, filepath.Base(path))
	}
	return out
}
