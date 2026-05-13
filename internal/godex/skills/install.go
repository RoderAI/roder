package skills

import (
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

type InstallOptions struct {
	Source      string
	Workspace   string
	DataDir     string
	Global      bool
	Project     bool
	SkillNames  []string
	Yes         bool
	TempDir     string
	CloneRunner func(context.Context, string, string) error
}

type InstallResult struct {
	Installed []InstalledSkill
}

type InstalledSkill struct {
	Name string
	Path string
}

type sourceSkill struct {
	Skill Skill
	Dir   string
}

func Install(ctx context.Context, opts InstallOptions) (InstallResult, error) {
	if strings.TrimSpace(opts.Source) == "" {
		return InstallResult{}, errors.New("source is required")
	}
	if opts.Global && opts.Project {
		return InstallResult{}, errors.New("choose only one of --global or --project")
	}
	sourceDir, cleanup, err := prepareSource(ctx, opts)
	if err != nil {
		return InstallResult{}, err
	}
	defer cleanup()

	found, err := sourceSkills(sourceDir)
	if err != nil {
		return InstallResult{}, err
	}
	selected := filterSourceSkills(found, opts.SkillNames)
	if len(selected) == 0 {
		return InstallResult{}, fmt.Errorf("no skills matched source %q", opts.Source)
	}
	if len(found) > 1 && len(opts.SkillNames) == 0 && !opts.Yes {
		return InstallResult{}, errors.New("source contains multiple skills; use --skill filters or --yes")
	}

	targetRoot, err := installRoot(opts)
	if err != nil {
		return InstallResult{}, err
	}
	var result InstallResult
	for _, item := range selected {
		target := filepath.Join(targetRoot, item.Skill.Name)
		if err := copySkillDir(item.Dir, target); err != nil {
			return InstallResult{}, err
		}
		result.Installed = append(result.Installed, InstalledSkill{Name: item.Skill.Name, Path: target})
	}
	return result, nil
}

func prepareSource(ctx context.Context, opts InstallOptions) (string, func(), error) {
	source := strings.TrimSpace(opts.Source)
	if !isGitSource(source) {
		abs, err := filepath.Abs(source)
		if err != nil {
			return "", func() {}, err
		}
		return abs, func() {}, nil
	}
	tempDir := opts.TempDir
	if tempDir == "" {
		var err error
		tempDir, err = os.MkdirTemp("", "gode-skill-*")
		if err != nil {
			return "", func() {}, err
		}
	} else if err := os.MkdirAll(tempDir, 0o755); err != nil {
		return "", func() {}, err
	}
	cleanup := func() {
		if opts.TempDir == "" {
			_ = os.RemoveAll(tempDir)
		}
	}
	clone := opts.CloneRunner
	if clone == nil {
		clone = gitClone
	}
	if err := clone(ctx, source, tempDir); err != nil {
		cleanup()
		return "", func() {}, err
	}
	return tempDir, cleanup, nil
}

func gitClone(ctx context.Context, source string, target string) error {
	cmd := exec.CommandContext(ctx, "git", "clone", "--depth=1", source, target)
	out, err := cmd.CombinedOutput()
	if err != nil {
		return fmt.Errorf("git clone: %w\n%s", err, strings.TrimSpace(string(out)))
	}
	return nil
}

func isGitSource(source string) bool {
	return strings.HasPrefix(source, "git@") ||
		strings.HasPrefix(source, "ssh://") ||
		strings.HasPrefix(source, "https://") ||
		strings.HasPrefix(source, "http://") ||
		strings.HasSuffix(source, ".git")
}

func sourceSkills(sourceDir string) ([]sourceSkill, error) {
	if _, err := os.Stat(filepath.Join(sourceDir, "SKILL.md")); err == nil {
		skill, err := ParseFile(filepath.Join(sourceDir, "SKILL.md"))
		if err != nil {
			return nil, err
		}
		return []sourceSkill{{Skill: skill, Dir: sourceDir}}, nil
	}
	entries, err := os.ReadDir(sourceDir)
	if err != nil {
		return nil, fmt.Errorf("read source %s: %w", sourceDir, err)
	}
	var out []sourceSkill
	for _, entry := range entries {
		if !entry.IsDir() {
			continue
		}
		dir := filepath.Join(sourceDir, entry.Name())
		skillPath := filepath.Join(dir, "SKILL.md")
		if _, err := os.Stat(skillPath); os.IsNotExist(err) {
			continue
		}
		skill, err := ParseFile(skillPath)
		if err != nil {
			return nil, err
		}
		out = append(out, sourceSkill{Skill: skill, Dir: dir})
	}
	return out, nil
}

func filterSourceSkills(skills []sourceSkill, names []string) []sourceSkill {
	if len(names) == 0 {
		return skills
	}
	wanted := map[string]struct{}{}
	for _, name := range names {
		if trimmed := strings.TrimSpace(name); trimmed != "" {
			wanted[trimmed] = struct{}{}
		}
	}
	var out []sourceSkill
	for _, skill := range skills {
		if _, ok := wanted[skill.Skill.Name]; ok {
			out = append(out, skill)
		}
	}
	return out
}

func installRoot(opts InstallOptions) (string, error) {
	if opts.Global {
		if strings.TrimSpace(opts.DataDir) == "" {
			return "", errors.New("data dir is required for global skill install")
		}
		return filepath.Join(opts.DataDir, "skills"), nil
	}
	if strings.TrimSpace(opts.Workspace) == "" {
		return "", errors.New("workspace is required for project skill install")
	}
	return filepath.Join(opts.Workspace, ".agents", "skills"), nil
}

func copySkillDir(source string, target string) error {
	if err := os.RemoveAll(target); err != nil {
		return err
	}
	if err := os.MkdirAll(target, 0o755); err != nil {
		return err
	}
	return filepath.WalkDir(source, func(path string, entry os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		rel, err := filepath.Rel(source, path)
		if err != nil || rel == "." {
			return err
		}
		targetPath := filepath.Join(target, rel)
		info, err := entry.Info()
		if err != nil {
			return err
		}
		mode := info.Mode()
		if mode&os.ModeSymlink != 0 {
			return nil
		}
		if entry.IsDir() {
			return os.MkdirAll(targetPath, mode.Perm())
		}
		return copyFile(path, targetPath, mode.Perm())
	})
}

func copyFile(source string, target string, mode os.FileMode) error {
	if err := os.MkdirAll(filepath.Dir(target), 0o755); err != nil {
		return err
	}
	in, err := os.Open(source)
	if err != nil {
		return err
	}
	defer in.Close()
	out, err := os.OpenFile(target, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, mode)
	if err != nil {
		return err
	}
	if _, err := io.Copy(out, in); err != nil {
		_ = out.Close()
		return err
	}
	return out.Close()
}
