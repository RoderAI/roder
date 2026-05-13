package imageinput

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

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

func TestEncodeFileReturnsImageDataURL(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "pasted.png")
	if err := os.WriteFile(path, tinyPNG, 0o600); err != nil {
		t.Fatal(err)
	}

	image, err := EncodeFile(path)
	if err != nil {
		t.Fatalf("encode: %v", err)
	}
	if !strings.HasPrefix(image.URL, "data:image/png;base64,") {
		t.Fatalf("url = %q", image.URL)
	}
	if image.MIME != "image/png" {
		t.Fatalf("mime = %q", image.MIME)
	}
	if image.Bytes != len(tinyPNG) {
		t.Fatalf("bytes = %d", image.Bytes)
	}
}

func TestEncodeFileRejectsNonImages(t *testing.T) {
	path := filepath.Join(t.TempDir(), "notes.txt")
	if err := os.WriteFile(path, []byte("hello"), 0o600); err != nil {
		t.Fatal(err)
	}

	_, err := EncodeFile(path)
	if err == nil || !strings.Contains(err.Error(), "unsupported image") {
		t.Fatalf("err = %v", err)
	}
}
