package commands

import (
	"context"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

func TestLoadBuildsUserAndProjectCommandIDs(t *testing.T) {
	root := t.TempDir()
	home := filepath.Join(root, "home")
	workspace := filepath.Join(root, "workspace")
	mustWrite(t, filepath.Join(home, ".config", "gode", "commands", "refactor", "extract.md"), "Extract $TARGET")
	mustWrite(t, filepath.Join(home, ".gode", "commands", "review.md"), "Review")
	mustWrite(t, filepath.Join(workspace, ".gode", "commands", "test.md"), "Run tests")

	catalog, err := Load(LoadOptions{Workspace: workspace, HomeDir: home})
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	if got := commandIDs(catalog.Commands); !reflect.DeepEqual(got, []string{"user:refactor:extract", "user:review", "project:test"}) {
		t.Fatalf("ids = %#v", got)
	}
	if !reflect.DeepEqual(catalog.Commands[0].Placeholders, []string{"TARGET"}) {
		t.Fatalf("placeholders = %#v", catalog.Commands[0].Placeholders)
	}
}

func TestExpandCommandPromptAndRequiresPlaceholders(t *testing.T) {
	catalog := Catalog{Commands: []Command{{
		ID:           "project:test",
		Scope:        "project",
		Prompt:       "Run $TARGET tests",
		Placeholders: []string{"TARGET"},
	}}}
	if _, err := Expand(context.Background(), "/test please", catalog); err == nil {
		t.Fatal("expected missing placeholder error")
	}
	result, err := Expand(context.Background(), "/test TARGET=api with coverage", catalog)
	if err != nil {
		t.Fatalf("expand: %v", err)
	}
	if result.Command == nil || result.Command.ID != "project:test" {
		t.Fatalf("command = %#v", result.Command)
	}
	if result.Prompt != "Run api tests\n\nwith coverage" {
		t.Fatalf("prompt = %q", result.Prompt)
	}
}

func TestExpandLeavesUnknownCommandsAndAbsolutePathsAlone(t *testing.T) {
	catalog := Catalog{Commands: []Command{{ID: "project:test", Scope: "project", Prompt: "Run tests"}}}
	for _, prompt := range []string{"/unknown do work", "/Users/pz/file.go should stay"} {
		result, err := Expand(context.Background(), prompt, catalog)
		if err != nil {
			t.Fatalf("expand %q: %v", prompt, err)
		}
		if result.Prompt != prompt || result.Command != nil {
			t.Fatalf("result for %q = %#v", prompt, result)
		}
	}
}

func TestExpandPrefersProjectCommandForShortIDs(t *testing.T) {
	catalog := Catalog{Commands: []Command{
		{ID: "user:test", Scope: "user", Prompt: "User"},
		{ID: "project:test", Scope: "project", Prompt: "Project"},
	}}
	result, err := Expand(context.Background(), "/test", catalog)
	if err != nil {
		t.Fatalf("expand: %v", err)
	}
	if result.Prompt != "Project" {
		t.Fatalf("prompt = %q", result.Prompt)
	}
}

func commandIDs(commands []Command) []string {
	out := make([]string, 0, len(commands))
	for _, command := range commands {
		out = append(out, command.ID)
	}
	return out
}

func mustWrite(t *testing.T, path string, data string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(path, []byte(strings.TrimSpace(data)), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}
}
