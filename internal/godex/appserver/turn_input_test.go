package appserver

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestBuildTurnInputCombinesTextAndImages(t *testing.T) {
	dir := t.TempDir()
	imagePath := filepath.Join(dir, "shot.png")
	if err := os.WriteFile(imagePath, tinyPNG, 0o600); err != nil {
		t.Fatal(err)
	}

	input := rawInput(t,
		map[string]any{"type": "text", "text": "describe this"},
		map[string]any{"type": "local_image", "path": imagePath},
		map[string]any{"type": "image", "image_url": "https://example.test/remote.png"},
	)
	built, err := buildTurnInput("", input)
	if err != nil {
		t.Fatalf("build: %v", err)
	}
	if built.Prompt != "describe this" {
		t.Fatalf("prompt = %q", built.Prompt)
	}
	if !built.ReplacePrompt {
		t.Fatalf("ReplacePrompt = false")
	}
	if len(built.InputItems) != 1 {
		t.Fatalf("items = %#v", built.InputItems)
	}
	item := built.InputItems[0]
	if item.Text != "describe this" || item.Role != "user" {
		t.Fatalf("item = %#v", item)
	}
	if len(item.Images) != 2 {
		t.Fatalf("images = %#v", item.Images)
	}
	if !strings.HasPrefix(item.Images[0].URL, "data:image/png;base64,") {
		t.Fatalf("local image url = %q", item.Images[0].URL)
	}
	if item.Images[1].URL != "https://example.test/remote.png" {
		t.Fatalf("remote image url = %q", item.Images[1].URL)
	}
}

func TestBuildTurnInputAddsLocalFileContent(t *testing.T) {
	dir := t.TempDir()
	filePath := filepath.Join(dir, "notes.md")
	if err := os.WriteFile(filePath, []byte("# Notes\n\nship attachments\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	input := rawInput(t,
		map[string]any{"type": "text", "text": "review this"},
		map[string]any{"type": "local_file", "path": filePath},
	)
	built, err := buildTurnInput("", input)
	if err != nil {
		t.Fatalf("build: %v", err)
	}
	if built.ReplacePrompt {
		t.Fatalf("ReplacePrompt = true")
	}
	for _, want := range []string{"review this", "Attached file: notes.md", "Path: " + filePath, "ship attachments"} {
		if !strings.Contains(built.Prompt, want) {
			t.Fatalf("prompt missing %q:\n%s", want, built.Prompt)
		}
	}
}

func TestBuildTurnInputUsesLocalFileImagesAsImageInput(t *testing.T) {
	dir := t.TempDir()
	imagePath := filepath.Join(dir, "shot.png")
	if err := os.WriteFile(imagePath, tinyPNG, 0o600); err != nil {
		t.Fatal(err)
	}

	input := rawInput(t, map[string]any{"type": "local_file", "path": imagePath})
	built, err := buildTurnInput("", input)
	if err != nil {
		t.Fatalf("build: %v", err)
	}
	if !built.ReplacePrompt {
		t.Fatalf("ReplacePrompt = false")
	}
	if len(built.InputItems) != 1 || len(built.InputItems[0].Images) != 1 {
		t.Fatalf("items = %#v", built.InputItems)
	}
	if !strings.Contains(built.Prompt, "Attached image: "+imagePath) {
		t.Fatalf("prompt = %q", built.Prompt)
	}
	if !strings.HasPrefix(built.InputItems[0].Images[0].URL, "data:image/png;base64,") {
		t.Fatalf("image url = %q", built.InputItems[0].Images[0].URL)
	}
}

func TestBuildTurnInputAddsBinaryLocalFileMetadata(t *testing.T) {
	dir := t.TempDir()
	filePath := filepath.Join(dir, "archive.bin")
	if err := os.WriteFile(filePath, []byte{0, 1, 2, 3}, 0o600); err != nil {
		t.Fatal(err)
	}

	input := rawInput(t, map[string]any{"type": "local_file", "path": filePath})
	built, err := buildTurnInput("", input)
	if err != nil {
		t.Fatalf("build: %v", err)
	}
	for _, want := range []string{"Attached file: archive.bin", "Content omitted because the file appears to be binary."} {
		if !strings.Contains(built.Prompt, want) {
			t.Fatalf("prompt missing %q:\n%s", want, built.Prompt)
		}
	}
}

func TestBuildTurnInputCollectsStructuredSkillSelections(t *testing.T) {
	skillPath := filepath.Join(t.TempDir(), "SKILL.md")
	input := rawInput(t,
		map[string]any{"type": "text", "text": "use this skill"},
		map[string]any{"type": "skill", "path": skillPath},
		map[string]any{"type": "skill", "name": "go-tests"},
	)
	built, err := buildTurnInput("", input)
	if err != nil {
		t.Fatalf("build: %v", err)
	}
	if built.Prompt != "use this skill" {
		t.Fatalf("prompt = %q", built.Prompt)
	}
	if len(built.SkillSelections) != 2 {
		t.Fatalf("selections = %#v", built.SkillSelections)
	}
	if built.SkillSelections[0].Path != skillPath || built.SkillSelections[1].Name != "go-tests" {
		t.Fatalf("selections = %#v", built.SkillSelections)
	}
}

func rawInput(t *testing.T, values ...map[string]any) []json.RawMessage {
	t.Helper()
	out := make([]json.RawMessage, 0, len(values))
	for _, value := range values {
		data, err := json.Marshal(value)
		if err != nil {
			t.Fatal(err)
		}
		out = append(out, data)
	}
	return out
}

var tinyPNG = []byte{
	0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a,
	0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
	0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
	0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
	0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41,
	0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
	0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00,
	0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae,
	0x42, 0x60, 0x82,
}
