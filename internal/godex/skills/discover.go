package skills

import (
	"os"
	"path/filepath"
	"strings"
)

type DiscoverOptions struct {
	Workspace string
	DataDir   string
	HomeDir   string
	Env       []string
}

type Catalog struct {
	Skills      []Skill
	Diagnostics []Diagnostic
}

func Discover(opts DiscoverOptions) Catalog {
	workspace := absOrDefault(opts.Workspace, ".")
	dataDir := strings.TrimSpace(opts.DataDir)
	homeDir := strings.TrimSpace(opts.HomeDir)
	if homeDir == "" {
		homeDir, _ = os.UserHomeDir()
	}

	roots := []string{filepath.Join(workspace, ".agents", "skills")}
	roots = append(roots, nestedAgentSkillRoots(workspace)...)
	roots = append(roots, filepath.Join(workspace, ".gode", "skills"))
	if dataDir != "" {
		roots = append(roots, filepath.Join(dataDir, "skills"))
	}
	if homeDir != "" {
		roots = append(roots, filepath.Join(homeDir, ".gode", "skills"))
	}
	if codexHome := envValue(opts.Env, "CODEX_HOME"); codexHome != "" {
		roots = append(roots, filepath.Join(codexHome, "skills"))
	}

	seenRoots := map[string]struct{}{}
	seenSkills := map[string]struct{}{}
	var catalog Catalog
	for _, root := range roots {
		abs := absOrDefault(root, root)
		if _, ok := seenRoots[abs]; ok {
			continue
		}
		seenRoots[abs] = struct{}{}
		discoverRoot(abs, &catalog, seenSkills)
	}
	return catalog
}

func discoverRoot(root string, catalog *Catalog, seenSkills map[string]struct{}) {
	entries, err := os.ReadDir(root)
	if os.IsNotExist(err) {
		return
	}
	if err != nil {
		catalog.Diagnostics = append(catalog.Diagnostics, Diagnostic{Path: root, Message: err.Error()})
		return
	}
	for _, entry := range entries {
		if !entry.IsDir() {
			continue
		}
		skillPath := filepath.Join(root, entry.Name(), "SKILL.md")
		skill, err := ParseFile(skillPath)
		if os.IsNotExist(err) {
			continue
		}
		if err != nil {
			catalog.Diagnostics = append(catalog.Diagnostics, Diagnostic{Path: skillPath, Message: err.Error()})
			continue
		}
		if _, exists := seenSkills[skill.Name]; exists {
			continue
		}
		seenSkills[skill.Name] = struct{}{}
		catalog.Skills = append(catalog.Skills, skill)
	}
}

func nestedAgentSkillRoots(workspace string) []string {
	var roots []string
	if workspace == "" {
		return roots
	}
	_ = filepath.WalkDir(workspace, func(path string, entry os.DirEntry, err error) error {
		if err != nil || !entry.IsDir() {
			return nil
		}
		rel, err := filepath.Rel(workspace, path)
		if err != nil || rel == "." {
			return nil
		}
		depth := len(strings.Split(filepath.ToSlash(rel), "/"))
		if depth > 5 {
			return filepath.SkipDir
		}
		if filepath.Base(path) == "skills" && filepath.Base(filepath.Dir(path)) == ".agents" {
			if filepath.Clean(path) != filepath.Join(workspace, ".agents", "skills") {
				roots = append(roots, path)
			}
			return filepath.SkipDir
		}
		return nil
	})
	return roots
}

func envValue(env []string, key string) string {
	if env == nil {
		env = os.Environ()
	}
	prefix := key + "="
	for _, item := range env {
		if strings.HasPrefix(item, prefix) {
			return strings.TrimSpace(strings.TrimPrefix(item, prefix))
		}
	}
	return ""
}

func absOrDefault(path string, fallback string) string {
	if strings.TrimSpace(path) == "" {
		path = fallback
	}
	abs, err := filepath.Abs(path)
	if err != nil {
		return path
	}
	return abs
}
