package attachments

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestAttachmentAppendTextContextAddsQuotedFile(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "README.md"), []byte("hello\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	prompt, err := AppendTextContext(root, "summarize", []Attachment{New("README.md")})
	if err != nil {
		t.Fatalf("append: %v", err)
	}
	for _, want := range []string{"summarize", "Attached context", "File README.md", "hello"} {
		if !strings.Contains(prompt, want) {
			t.Fatalf("prompt missing %q:\n%s", want, prompt)
		}
	}
}

func TestAttachmentAppendTextContextRejectsImages(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "image.png"), tinyPNG, 0o600); err != nil {
		t.Fatal(err)
	}

	input, err := BuildPromptInput(root, "look", []Attachment{New("image.png")})
	if err != nil {
		t.Fatalf("build input: %v", err)
	}
	if !input.ReplacePrompt {
		t.Fatalf("ReplacePrompt = false")
	}
	if len(input.Items) != 1 {
		t.Fatalf("items = %#v", input.Items)
	}
	item := input.Items[0]
	if item.Role != "user" || item.Text != "look" {
		t.Fatalf("item = %#v", item)
	}
	if len(item.Images) != 1 || !strings.HasPrefix(item.Images[0].URL, "data:image/png;base64,") {
		t.Fatalf("images = %#v", item.Images)
	}
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
