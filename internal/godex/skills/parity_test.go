package skills

import (
	"os"
	"path/filepath"
	"reflect"
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

func TestCodexParityCatalogComparisonCoversCatalogShape(t *testing.T) {
	path := filepath.Join(t.TempDir(), "go-tests", "SKILL.md")
	catalog := Catalog{
		Skills: []Skill{{
			Name:        "go-tests",
			Description: "Run Go tests",
			Path:        path,
			Scope:       SkillScopeRepo,
			Metadata:    map[string]string{"owner": "agents"},
			Interface:   &SkillInterface{DisplayName: "Go Tests"},
			Dependencies: &SkillDependencies{Tools: []SkillToolDependency{{
				Type:  "mcp",
				Value: "filesystem",
			}}},
		}},
		Diagnostics: []Diagnostic{{Path: filepath.Join(filepath.Dir(path), "bad", "SKILL.md"), Message: "invalid"}},
	}
	cfg := Config{Rules: []ConfigRule{{Path: path, Enabled: false}}}

	got, diagnostics := catalogSnapshot(catalog, cfg)
	assertCatalogSnapshot(t, got, []skillCatalogSnapshot{{
		Path:         canonicalPath(path),
		Scope:        SkillScopeRepo,
		Name:         "go-tests",
		Metadata:     map[string]string{"owner": "agents"},
		Enabled:      false,
		Interface:    SkillInterface{DisplayName: "Go Tests"},
		Dependencies: []SkillToolDependency{{Type: "mcp", Value: "filesystem"}},
		Diagnostics:  nil,
	}}, diagnostics, []Diagnostic{{Path: filepath.Join(filepath.Dir(path), "bad", "SKILL.md"), Message: "invalid"}})
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

type skillCatalogSnapshot struct {
	Path         string
	Scope        SkillScope
	Name         string
	Metadata     map[string]string
	Enabled      bool
	Interface    SkillInterface
	Dependencies []SkillToolDependency
	Diagnostics  []Diagnostic
}

func catalogSnapshot(catalog Catalog, config Config) ([]skillCatalogSnapshot, []Diagnostic) {
	out := make([]skillCatalogSnapshot, 0, len(catalog.Skills))
	for _, skill := range catalog.Skills {
		snapshot := skillCatalogSnapshot{
			Path:     skillIdentity(skill),
			Scope:    skill.Scope,
			Name:     skill.Name,
			Metadata: cloneStringMap(skill.Metadata),
			Enabled:  IsSkillEnabled(config, skill),
		}
		if skill.Interface != nil {
			snapshot.Interface = *skill.Interface
		}
		if skill.Dependencies != nil {
			snapshot.Dependencies = append([]SkillToolDependency(nil), skill.Dependencies.Tools...)
		}
		out = append(out, snapshot)
	}
	return out, append([]Diagnostic(nil), catalog.Diagnostics...)
}

func assertCatalogSnapshot(t *testing.T, got []skillCatalogSnapshot, want []skillCatalogSnapshot, gotDiagnostics []Diagnostic, wantDiagnostics []Diagnostic) {
	t.Helper()
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("catalog snapshot mismatch\n got: %#v\nwant: %#v", got, want)
	}
	if !reflect.DeepEqual(gotDiagnostics, wantDiagnostics) {
		t.Fatalf("diagnostics mismatch\n got: %#v\nwant: %#v", gotDiagnostics, wantDiagnostics)
	}
}

func cloneStringMap(in map[string]string) map[string]string {
	if len(in) == 0 {
		return nil
	}
	out := make(map[string]string, len(in))
	for key, value := range in {
		out[key] = value
	}
	return out
}
