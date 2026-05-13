package completions

import (
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
)

type FileItem struct {
	Path  string
	IsDir bool
}

func Files(workspace string, query string, limit int) []FileItem {
	workspace = absOrDefault(workspace, ".")
	query = strings.TrimPrefix(strings.TrimSpace(filepath.ToSlash(query)), "@")
	if limit <= 0 {
		limit = 50
	}
	ignored := gitIgnoreChecker(workspace)
	var items []FileItem
	_ = filepath.WalkDir(workspace, func(path string, entry os.DirEntry, err error) error {
		if err != nil {
			return nil
		}
		if path == workspace {
			return nil
		}
		rel, err := filepath.Rel(workspace, path)
		if err != nil {
			return nil
		}
		rel = filepath.ToSlash(rel)
		if entry.IsDir() {
			if entry.Name() == ".git" || ignored(rel+"/") {
				return filepath.SkipDir
			}
			return nil
		}
		if ignored(rel) || query != "" && !strings.Contains(strings.ToLower(rel), strings.ToLower(query)) {
			return nil
		}
		items = append(items, FileItem{Path: rel})
		return nil
	})
	sort.Slice(items, func(i, j int) bool { return items[i].Path < items[j].Path })
	if len(items) > limit {
		return items[:limit]
	}
	return items
}

func gitIgnoreChecker(workspace string) func(string) bool {
	if err := exec.Command("git", "-C", workspace, "rev-parse", "--is-inside-work-tree").Run(); err != nil {
		return func(string) bool { return false }
	}
	return func(rel string) bool {
		return exec.Command("git", "-C", workspace, "check-ignore", "-q", "--", rel).Run() == nil
	}
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
