package contextpack

import (
	"bytes"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"

	"github.com/pandelisz/gode/internal/godex/provider"
	"github.com/pandelisz/gode/internal/godex/repoconfig"
)

const DefaultMaxBytes = 128 * 1024

var DefaultContextPaths = []string{
	"AGENTS.md",
	"CLAUDE.md",
	"GEMINI.md",
	".cursorrules",
	filepath.Join(".github", "copilot-instructions.md"),
	"gode.md",
}

type LoadOptions struct {
	Workspace string
	Repo      repoconfig.Loaded
	MaxBytes  int
}

type Pack struct {
	Files []File
	Extra []ExtraContext
}

type File struct {
	Path      string
	RelPath   string
	Content   string
	Truncated bool
	BytesRead int64
	BytesOmit int64
}

type ExtraContext struct {
	Source  string
	Content string
}

func Load(opts LoadOptions) (Pack, error) {
	workspace, err := absDir(opts.Workspace)
	if err != nil {
		return Pack{}, err
	}
	maxBytes := opts.MaxBytes
	if maxBytes <= 0 {
		maxBytes = DefaultMaxBytes
	}

	refs := make([]fileRef, 0, len(DefaultContextPaths))
	for _, path := range DefaultContextPaths {
		refs = append(refs, fileRef{BaseDir: workspace, Path: path})
	}
	var extra []ExtraContext
	for _, cfg := range opts.Repo.Configs {
		if text := strings.TrimSpace(cfg.Agent.ExtraContext); text != "" {
			extra = append(extra, ExtraContext{Source: cfg.Path, Content: text})
		}
		for _, path := range cfg.Agent.ContextPaths {
			refs = append(refs, fileRef{BaseDir: cfg.Dir, Path: path})
		}
	}

	files, err := loadFiles(workspace, refs, int64(maxBytes))
	if err != nil {
		return Pack{}, err
	}
	return Pack{Files: files, Extra: extra}, nil
}

func (p Pack) Messages() []provider.Message {
	messages := make([]provider.Message, 0, len(p.Extra)+len(p.Files))
	for _, extra := range p.Extra {
		messages = append(messages, provider.Message{
			Role:    provider.RoleSystem,
			Content: fmt.Sprintf("<repo-context source=%q>\n%s\n</repo-context>", extra.Source, extra.Content),
		})
	}
	for _, file := range p.Files {
		messages = append(messages, provider.Message{
			Role:    provider.RoleSystem,
			Content: fmt.Sprintf("<repo-context-file path=%q>\n%s\n</repo-context-file>", file.RelPath, file.Content),
		})
	}
	return messages
}

type fileRef struct {
	BaseDir string
	Path    string
}

func loadFiles(workspace string, refs []fileRef, maxBytes int64) ([]File, error) {
	seen := map[string]struct{}{}
	var files []File
	for _, ref := range refs {
		path := strings.TrimSpace(ref.Path)
		if path == "" {
			continue
		}
		abs := path
		if !filepath.IsAbs(abs) {
			abs = filepath.Join(ref.BaseDir, path)
		}
		abs, err := filepath.Abs(abs)
		if err != nil {
			return nil, fmt.Errorf("context path %s: %w", path, err)
		}
		if _, ok := seen[abs]; ok {
			continue
		}
		seen[abs] = struct{}{}
		file, ok, err := readFile(workspace, abs, maxBytes)
		if err != nil {
			return nil, err
		}
		if ok {
			files = append(files, file)
		}
	}
	return files, nil
}

func readFile(workspace string, path string, maxBytes int64) (File, bool, error) {
	info, err := os.Stat(path)
	if os.IsNotExist(err) {
		return File{}, false, nil
	}
	if err != nil {
		return File{}, false, fmt.Errorf("stat context file %s: %w", path, err)
	}
	if info.IsDir() {
		return File{}, false, nil
	}
	file, err := os.Open(path)
	if err != nil {
		return File{}, false, fmt.Errorf("read context file %s: %w", path, err)
	}
	defer file.Close()
	data, err := io.ReadAll(io.LimitReader(file, maxBytes+1))
	if err != nil {
		return File{}, false, fmt.Errorf("read context file %s: %w", path, err)
	}
	bytesRead := info.Size()
	truncated := int64(len(data)) > maxBytes
	omitted := int64(0)
	if truncated {
		omitted = info.Size() - maxBytes
		data = data[:maxBytes]
		data = bytes.TrimRight(data, "\x00\r\n\t ")
		data = append(data, []byte(fmt.Sprintf("\n\n[truncated: omitted %d bytes from %d byte context file]", omitted, bytesRead))...)
	}
	rel, err := filepath.Rel(workspace, path)
	if err != nil || strings.HasPrefix(rel, ".."+string(filepath.Separator)) || rel == ".." {
		rel = path
	}
	return File{
		Path:      path,
		RelPath:   filepath.ToSlash(rel),
		Content:   string(data),
		Truncated: truncated,
		BytesRead: bytesRead,
		BytesOmit: omitted,
	}, true, nil
}

func absDir(path string) (string, error) {
	if strings.TrimSpace(path) == "" {
		path = "."
	}
	abs, err := filepath.Abs(path)
	if err != nil {
		return "", fmt.Errorf("workspace: %w", err)
	}
	info, err := os.Stat(abs)
	if err == nil && !info.IsDir() {
		abs = filepath.Dir(abs)
	}
	if err != nil && !os.IsNotExist(err) {
		return "", fmt.Errorf("stat workspace %s: %w", abs, err)
	}
	return abs, nil
}
