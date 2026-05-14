package memory

import (
	"strings"
	"testing"
	"time"
)

func TestMemoryRecallFormatCapsPreviewAndTotalText(t *testing.T) {
	long := strings.Repeat("semantic memory ", 300)
	entries := make([]Entry, 12)
	for i := range entries {
		entries[i] = Entry{
			ID:        "mem_" + string(rune('a'+i)),
			Content:   long,
			UpdatedAt: time.Now().UTC(),
		}
	}

	text := FormatRecallSnippets(entries)
	if !strings.Contains(text, "Relevant local memories for this workspace:") {
		t.Fatalf("text = %q", text)
	}
	if !strings.Contains(text, "Use read_memory with the memory ID") {
		t.Fatalf("text = %q", text)
	}
	if len(text) > MaxRecallSnippetBytes {
		t.Fatalf("snippet len = %d, want <= %d", len(text), MaxRecallSnippetBytes)
	}
	for _, line := range strings.Split(text, "\n") {
		if strings.Contains(line, "mem_") && len(line) > MaxRecallPreviewChars+32 {
			t.Fatalf("preview line too long (%d): %q", len(line), line)
		}
	}
}

func TestMemoryRecallFormatEmptyEntries(t *testing.T) {
	if got := FormatRecallSnippets(nil); got != "" {
		t.Fatalf("text = %q", got)
	}
}
