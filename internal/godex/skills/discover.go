package skills

import (
	"os"
	"path/filepath"
	"sort"
	"strings"
)

type DiscoverOptions struct {
	Workspace string
	DataDir   string
	HomeDir   string
	Env       []string
	Roots     []Root
}

type Root struct {
	Path     string
	Scope    SkillScope
	PluginID string
}

type Catalog struct {
	Skills      []Skill
	Diagnostics []Diagnostic
	Roots       []Root
}

func Discover(opts DiscoverOptions) Catalog {
	roots := discoverRoots(opts)
	seenRoots := map[string]struct{}{}
	seenSkills := map[string]struct{}{}
	var catalog Catalog
	for _, root := range roots {
		root.Path = canonicalPath(root.Path)
		if root.Path == "" {
			continue
		}
		if _, ok := seenRoots[root.Path]; ok {
			continue
		}
		seenRoots[root.Path] = struct{}{}
		before := len(catalog.Skills)
		discoverRoot(root, &catalog, seenSkills)
		if len(catalog.Skills) > before {
			catalog.Roots = append(catalog.Roots, root)
		}
	}
	sortCatalog(&catalog)
	return catalog
}

func discoverRoots(opts DiscoverOptions) []Root {
	workspace := absOrDefault(opts.Workspace, ".")
	dataDir := strings.TrimSpace(opts.DataDir)
	homeDir := strings.TrimSpace(opts.HomeDir)
	if homeDir == "" {
		homeDir, _ = os.UserHomeDir()
	}

	roots := []Root{{Path: filepath.Join(workspace, ".agents", "skills"), Scope: SkillScopeRepo}}
	for _, nested := range nestedAgentSkillRoots(workspace) {
		roots = append(roots, Root{Path: nested, Scope: SkillScopeRepo})
	}
	roots = append(roots, Root{Path: filepath.Join(workspace, ".gode", "skills"), Scope: SkillScopeRepo})
	if dataDir != "" {
		roots = append(roots, Root{Path: filepath.Join(dataDir, "skills"), Scope: SkillScopeUser})
	}
	if homeDir != "" {
		roots = append(roots, Root{Path: filepath.Join(homeDir, ".agents", "skills"), Scope: SkillScopeUser})
		roots = append(roots, Root{Path: filepath.Join(homeDir, ".codex", "skills"), Scope: SkillScopeUser})
		roots = append(roots, Root{Path: filepath.Join(homeDir, ".gode", "skills"), Scope: SkillScopeUser})
	}
	if codexHome := envValue(opts.Env, "CODEX_HOME"); codexHome != "" {
		roots = append(roots, Root{Path: filepath.Join(codexHome, "skills"), Scope: SkillScopeUser})
	}
	for _, path := range filepath.SplitList(envValue(opts.Env, "GODE_SYSTEM_SKILLS")) {
		if strings.TrimSpace(path) != "" {
			roots = append(roots, Root{Path: path, Scope: SkillScopeSystem})
		}
	}
	for _, path := range filepath.SplitList(envValue(opts.Env, "GODE_ADMIN_SKILLS")) {
		if strings.TrimSpace(path) != "" {
			roots = append(roots, Root{Path: path, Scope: SkillScopeAdmin})
		}
	}
	roots = append(roots, opts.Roots...)
	return roots
}

func discoverRoot(root Root, catalog *Catalog, seenSkills map[string]struct{}) {
	info, err := os.Stat(root.Path)
	if os.IsNotExist(err) {
		return
	}
	if err != nil {
		catalog.Diagnostics = append(catalog.Diagnostics, Diagnostic{Path: root.Path, Message: err.Error()})
		return
	}
	if !info.IsDir() {
		return
	}
	visited := 0
	walk(root.Path, root.Path, root, catalog, seenSkills, 0, &visited)
}

func walk(rootPath, dir string, root Root, catalog *Catalog, seenSkills map[string]struct{}, depth int, visited *int) {
	if depth > defaultMaxScanDepth || *visited >= defaultMaxSkillDirsPerRoot {
		return
	}
	*visited++
	if depth > 0 && strings.HasPrefix(filepath.Base(dir), ".") {
		return
	}
	skillPath := filepath.Join(dir, skillFileName)
	if info, err := os.Stat(skillPath); err == nil && !info.IsDir() {
		addSkill(rootPath, skillPath, root, catalog, seenSkills)
		return
	}

	entries, err := os.ReadDir(dir)
	if err != nil {
		catalog.Diagnostics = append(catalog.Diagnostics, Diagnostic{Path: dir, Message: err.Error()})
		return
	}
	for _, entry := range entries {
		name := entry.Name()
		if strings.HasPrefix(name, ".") {
			continue
		}
		path := filepath.Join(dir, name)
		isDir := entry.IsDir()
		if !isDir && entry.Type()&os.ModeSymlink != 0 && root.Scope != SkillScopeSystem {
			if info, err := os.Stat(path); err == nil && info.IsDir() {
				isDir = true
			}
		}
		if !isDir {
			continue
		}
		walk(rootPath, path, root, catalog, seenSkills, depth+1, visited)
	}
}

func addSkill(rootPath, skillPath string, root Root, catalog *Catalog, seenSkills map[string]struct{}) {
	key := canonicalPath(skillPath)
	if _, exists := seenSkills[key]; exists {
		return
	}
	skill, err := ParseFile(skillPath)
	if err != nil {
		catalog.Diagnostics = append(catalog.Diagnostics, Diagnostic{Path: skillPath, Message: err.Error()})
		return
	}
	seenSkills[key] = struct{}{}
	skill.Path = key
	skill.Scope = root.Scope
	skill.PluginID = root.PluginID
	skill.Root = canonicalPath(rootPath)
	catalog.Skills = append(catalog.Skills, skill)
}

func sortCatalog(catalog *Catalog) {
	sort.Slice(catalog.Skills, func(i, j int) bool {
		a, b := catalog.Skills[i], catalog.Skills[j]
		if rank := scopeRank(a.Scope) - scopeRank(b.Scope); rank != 0 {
			return rank < 0
		}
		if a.Name != b.Name {
			return a.Name < b.Name
		}
		return a.Path < b.Path
	})
	sort.Slice(catalog.Diagnostics, func(i, j int) bool {
		return catalog.Diagnostics[i].Path < catalog.Diagnostics[j].Path
	})
}

func scopeRank(scope SkillScope) int {
	switch scope {
	case SkillScopeRepo:
		return 0
	case SkillScopeUser:
		return 1
	case SkillScopeSystem:
		return 2
	case SkillScopeAdmin:
		return 3
	default:
		return 4
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
		if path != workspace && strings.HasPrefix(entry.Name(), ".") && entry.Name() != ".agents" {
			return filepath.SkipDir
		}
		rel, err := filepath.Rel(workspace, path)
		if err != nil || rel == "." {
			return nil
		}
		depth := len(strings.Split(filepath.ToSlash(rel), "/"))
		if depth > defaultMaxScanDepth {
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
