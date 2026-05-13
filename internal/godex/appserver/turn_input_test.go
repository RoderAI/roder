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
