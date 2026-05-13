package contextpack

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex/provider"
	"github.com/pandelisz/gode/internal/godex/repoconfig"
)

func TestLoadDefaultAndConfiguredContextDedupesByAbsolutePath(t *testing.T) {
	workspace := t.TempDir()
	mustWrite(t, filepath.Join(workspace, "AGENTS.md"), "agent rules")
	mustWrite(t, filepath.Join(workspace, "docs", "extra.md"), "extra context")

	repo := repoconfig.Loaded{Configs: []repoconfig.Config{{
		Path: filepath.Join(workspace, ".gode.toml"),
		Dir:  workspace,
		Agent: repoconfig.AgentConfig{
			ExtraContext: "inline context",
			ContextPaths: []string{"AGENTS.md", "docs/extra.md", "missing.md"},
		},
	}}}
	pack, err := Load(LoadOptions{Workspace: workspace, Repo: repo})
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	if len(pack.Files) != 2 {
		t.Fatalf("files = %#v", pack.Files)
	}
	if pack.Files[0].RelPath != "AGENTS.md" || pack.Files[1].RelPath != "docs/extra.md" {
		t.Fatalf("files = %#v", pack.Files)
	}
	if len(pack.Extra) != 1 || pack.Extra[0].Content != "inline context" {
		t.Fatalf("extra = %#v", pack.Extra)
	}
}

func TestLoadTruncatesOversizedFilesWithVisibleNote(t *testing.T) {
	workspace := t.TempDir()
	mustWrite(t, filepath.Join(workspace, "AGENTS.md"), strings.Repeat("a", 20))

	pack, err := Load(LoadOptions{Workspace: workspace, MaxBytes: 8})
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	if len(pack.Files) != 1 {
		t.Fatalf("files = %#v", pack.Files)
	}
	file := pack.Files[0]
	if !file.Truncated || file.BytesRead != 20 || file.BytesOmit != 12 {
		t.Fatalf("file = %#v", file)
	}
	if !strings.Contains(file.Content, "[truncated: omitted 12 bytes from 20 byte context file]") {
		t.Fatalf("content missing note:\n%s", file.Content)
	}
}

func TestMessagesUseHiddenSystemRole(t *testing.T) {
	pack := Pack{
		Extra: []ExtraContext{{Source: "/repo/.gode.toml", Content: "inline"}},
		Files: []File{{
			RelPath: "AGENTS.md",
			Content: "rules",
		}},
	}
	messages := pack.Messages()
	if len(messages) != 2 {
		t.Fatalf("messages = %#v", messages)
	}
	for _, msg := range messages {
		if msg.Role != provider.RoleSystem {
			t.Fatalf("message role = %q", msg.Role)
		}
	}
	if !strings.Contains(messages[0].Content, "<repo-context") || !strings.Contains(messages[1].Content, "<repo-context-file") {
		t.Fatalf("messages = %#v", messages)
	}
}

func mustWrite(t *testing.T, path string, data string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(path, []byte(data), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}
}
