package selection

import "testing"

func TestClipboardWriterFunc(t *testing.T) {
	var got string
	writer := ClipboardWriter(func(text string) error {
		got = text
		return nil
	})

	if err := writer("copied"); err != nil {
		t.Fatalf("writer: %v", err)
	}
	if got != "copied" {
		t.Fatalf("clipboard text = %q", got)
	}
}
