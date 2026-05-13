package diffview

import (
	"strings"
	"testing"

	"charm.land/lipgloss/v2"
)

func TestRenderLinesCapsDiffWithoutChangingWidth(t *testing.T) {
	diff := strings.Join([]string{
		"diff --git a/main.go b/main.go",
		"@@ -1 +1 @@",
		"-old",
		"+new",
		"context",
	}, "\n")
	lines := RenderLines(diff, 20, 3)
	if len(lines) != 4 {
		t.Fatalf("lines = %#v", lines)
	}
	if !strings.Contains(lines[len(lines)-1], "diff truncated") {
		t.Fatalf("missing truncation note: %#v", lines)
	}
	for _, line := range lines {
		if lipgloss.Width(line) > 20 {
			t.Fatalf("line unexpectedly wide: %q", line)
		}
	}
}

func TestIsDiffTool(t *testing.T) {
	for _, tool := range []string{"git_diff", "edit", "multi_edit", "write_file"} {
		if !IsDiffTool(tool) {
			t.Fatalf("%s should be a diff tool", tool)
		}
	}
	if IsDiffTool("read_file") {
		t.Fatal("read_file should not render as a diff tool")
	}
}
