package repoconfig

import (
	"os"
	"path/filepath"
	"reflect"
	"testing"
)

func TestLoadWalksUpwardAndStopsAtTopmostConfig(t *testing.T) {
	root := t.TempDir()
	mustWrite(t, filepath.Join(root, "gode.toml"), `[agent]
model = "root-model"
context_paths = ["root.md"]
`)
	project := filepath.Join(root, "project")
	mustWrite(t, filepath.Join(project, ".gode.toml"), `is_topmost_config = true
[agent]
model = "project-model"
extra_context = "project context"
context_paths = ["project.md"]
[hooks]
on_save = ["go test ./..."]
`)
	nested := filepath.Join(project, "src")
	mustWrite(t, filepath.Join(nested, "gode.toml"), `[agent]
context_paths = ["src.md"]
`)

	loaded, err := Load(nested)
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	if got := configBaseNames(loaded.Configs); !reflect.DeepEqual(got, []string{".gode.toml", "gode.toml"}) {
		t.Fatalf("configs = %#v", got)
	}
	if loaded.Model() != "project-model" {
		t.Fatalf("model = %q", loaded.Model())
	}
	if got := loaded.Configs[0].Hooks.OnSave; !reflect.DeepEqual(got, []string{"go test ./..."}) {
		t.Fatalf("hooks = %#v", got)
	}
}

func TestLoadSupportsBothNativeNamesInOneDirectory(t *testing.T) {
	root := t.TempDir()
	mustWrite(t, filepath.Join(root, ".gode.toml"), `[agent]
context_paths = ["a.md"]
`)
	mustWrite(t, filepath.Join(root, "gode.toml"), `[agent]
context_paths = ["b.md"]
`)

	loaded, err := Load(root)
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	if got := configBaseNames(loaded.Configs); !reflect.DeepEqual(got, []string{".gode.toml", "gode.toml"}) {
		t.Fatalf("configs = %#v", got)
	}
}

func configBaseNames(configs []Config) []string {
	out := make([]string, 0, len(configs))
	for _, cfg := range configs {
		out = append(out, filepath.Base(cfg.Path))
	}
	return out
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
