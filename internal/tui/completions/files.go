package completions

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"
)

const fileCompletionCacheTTL = 2 * time.Second

type FileItem struct {
	Path  string
	IsDir bool
}

type fileCompletionCacheEntry struct {
	workspace string
	items     []FileItem
	expires   time.Time
}

var fileCompletionCache struct {
	sync.Mutex
	entry fileCompletionCacheEntry
}

func Files(workspace string, query string, limit int) []FileItem {
	workspace = absOrDefault(workspace, ".")
	query = strings.TrimPrefix(strings.TrimSpace(filepath.ToSlash(query)), "@")
	query = strings.ToLower(query)
	if limit <= 0 {
		limit = 50
	}

	all := cachedWorkspaceFiles(workspace)
	items := make([]FileItem, 0, min(limit, len(all)))
	for _, item := range all {
		if query != "" && !strings.Contains(strings.ToLower(item.Path), query) {
			continue
		}
		items = append(items, item)
		if len(items) >= limit {
			break
		}
	}
	return items
}

func cachedWorkspaceFiles(workspace string) []FileItem {
	now := time.Now()
	fileCompletionCache.Lock()
	if fileCompletionCache.entry.workspace == workspace && now.Before(fileCompletionCache.entry.expires) {
		items := cloneFileItems(fileCompletionCache.entry.items)
		fileCompletionCache.Unlock()
		return items
	}
	fileCompletionCache.Unlock()

	items := listWorkspaceFiles(workspace)

	fileCompletionCache.Lock()
	fileCompletionCache.entry = fileCompletionCacheEntry{
		workspace: workspace,
		items:     cloneFileItems(items),
		expires:   now.Add(fileCompletionCacheTTL),
	}
	fileCompletionCache.Unlock()
	return items
}

func listWorkspaceFiles(workspace string) []FileItem {
	if items, ok := gitFiles(workspace); ok {
		return items
	}
	return walkFiles(workspace)
}

func gitFiles(workspace string) ([]FileItem, bool) {
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	cmd := exec.CommandContext(ctx, "git", "-C", workspace, "ls-files", "-co", "--exclude-standard", "-z")
	out, err := cmd.Output()
	if err != nil || ctx.Err() != nil {
		return nil, false
	}

	paths := strings.Split(string(out), "\x00")
	items := make([]FileItem, 0, len(paths))
	for _, path := range paths {
		if path == "" {
			continue
		}
		items = append(items, FileItem{Path: filepath.ToSlash(path)})
	}
	sort.Slice(items, func(i, j int) bool { return items[i].Path < items[j].Path })
	return items, true
}

func walkFiles(workspace string) []FileItem {
	var items []FileItem
	_ = filepath.WalkDir(workspace, func(path string, entry os.DirEntry, err error) error {
		if err != nil {
			return nil
		}
		if path == workspace {
			return nil
		}
		if entry.IsDir() {
			if shouldSkipCompletionDir(entry.Name()) {
				return filepath.SkipDir
			}
			return nil
		}
		rel, err := filepath.Rel(workspace, path)
		if err != nil {
			return nil
		}
		items = append(items, FileItem{Path: filepath.ToSlash(rel)})
		return nil
	})
	sort.Slice(items, func(i, j int) bool { return items[i].Path < items[j].Path })
	return items
}

func shouldSkipCompletionDir(name string) bool {
	switch name {
	case ".git", "node_modules", "vendor", ".idea", ".vscode", ".cache", "dist", "build":
		return true
	default:
		return false
	}
}

func cloneFileItems(items []FileItem) []FileItem {
	if len(items) == 0 {
		return nil
	}
	out := make([]FileItem, len(items))
	copy(out, items)
	return out
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
