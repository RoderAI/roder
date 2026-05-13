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
	_, err := AppendTextContext(t.TempDir(), "look", []Attachment{New("image.png")})
	if err == nil || !strings.Contains(err.Error(), "image attachments are not supported") {
		t.Fatalf("err = %v", err)
	}
}
