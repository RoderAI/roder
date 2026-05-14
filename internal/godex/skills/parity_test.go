package skills

import (
	"os"
	"path/filepath"
	"testing"
)

func TestCodexParityFixturesCoverCoreContracts(t *testing.T) {
	root := t.TempDir()
	writeSkill(t, filepath.Join(root, "repo", "duplicate"), "duplicate", "Repo duplicate")
	writeSkill(t, filepath.Join(root, "user", "duplicate"), "duplicate", "User duplicate")
	mustWrite(t, filepath.Join(root, "repo", ".hidden", "hidden", "SKILL.md"), "---\nname: hidden\ndescription: hidden\n---\n")
	mustWrite(t, filepath.Join(root, "repo", "bad", "SKILL.md"), "---\nname: bad_name\ndescription: bad\n---\n")

	catalog := Discover(DiscoverOptions{Roots: []Root{
		{Path: filepath.Join(root, "repo"), Scope: SkillScopeRepo},
		{Path: filepath.Join(root, "user"), Scope: SkillScopeUser},
	}, HomeDir: filepath.Join(root, "home")})
	if got := skillNames(catalog.Skills); len(got) != 2 || got[0] != "duplicate" || got[1] != "duplicate" {
		t.Fatalf("skills = %#v diagnostics=%#v", got, catalog.Diagnostics)
	}
	if catalog.Skills[0].Path == catalog.Skills[1].Path {
		t.Fatalf("duplicates should be preserved by path: %#v", catalog.Skills)
	}
	if len(catalog.Diagnostics) != 1 {
		t.Fatalf("diagnostics = %#v", catalog.Diagnostics)
	}
}

func TestCodexReferenceParity(t *testing.T) {
	reference := os.Getenv("CODEX_REFERENCE_DIR")
	if reference == "" {
		t.Skip("set CODEX_REFERENCE_DIR to run reference parity smoke")
	}
	for _, rel := range []string{
		"codex-rs/core-skills/src/model.rs",
		"codex-rs/core-skills/src/loader.rs",
		"codex-rs/core-skills/src/render.rs",
		"codex-rs/core-skills/src/injection.rs",
		"codex-rs/core-skills/src/config_rules.rs",
	} {
		if _, err := os.Stat(filepath.Join(reference, rel)); err != nil {
			t.Fatalf("reference file %s: %v", rel, err)
		}
	}
}
