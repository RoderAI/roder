package selection

import (
	"strings"
	"testing"
)

func TestSanitizeTranscriptCopyRemovesDecorativeChrome(t *testing.T) {
	lines := []TranscriptLineRef{
		{Text: "USER"},
		{Text: "hello from the transcript"},
		{Text: "ASSISTANT"},
		{Text: "┌────────────┐"},
		{Text: "│ decorative │", Decorative: true},
		{Text: "... 90 lines collapsed"},
		{Text: "ctrl+l errors"},
		{Text: "errors 1  ctx 90%  scroll 0"},
		{Text: "├ tree connector", Decorative: true},
		{Text: "> prompt prefix", Decorative: true},
		{Text: "\x1b[35mTOOL\x1b[0m"},
		{Text: "rendered shell row", CopyText: "clean shell text"},
	}

	got := SanitizeTranscriptCopy(lines)
	want := strings.Join([]string{
		"hello from the transcript",
		"clean shell text",
	}, "\n")
	if got != want {
		t.Fatalf("sanitized copy:\n got: %q\nwant: %q", got, want)
	}
}

func TestSanitizeTranscriptCopyKeepsMeaningfulContent(t *testing.T) {
	lines := []TranscriptLineRef{
		{Text: "- keep markdown bullet"},
		{Text: "```go"},
		{Text: "fmt.Println(\"hi\")"},
		{Text: "```"},
		{Text: "[docs](https://example.com/docs)"},
		{Text: "https://example.com/path"},
		{Text: "$ go test ./..."},
		{Text: "> quoted shell output"},
	}

	got := SanitizeTranscriptCopy(lines)
	want := strings.Join([]string{
		"- keep markdown bullet",
		"```go",
		"fmt.Println(\"hi\")",
		"```",
		"[docs](https://example.com/docs)",
		"https://example.com/path",
		"$ go test ./...",
		"> quoted shell output",
	}, "\n")
	if got != want {
		t.Fatalf("sanitized meaningful content:\n got: %q\nwant: %q", got, want)
	}
}
