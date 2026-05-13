package builtin

import (
	"bufio"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"

	"github.com/pandelisz/gode/internal/godex/permission"
	"github.com/pandelisz/gode/internal/godex/tools"
)

func RegisterFilesystem(reg *tools.Registry, root string) {
	reg.Register(tools.Tool{
		Name:          "read_file",
		Description:   "Read a UTF-8 text file inside the workspace.",
		ReadOnly:      true,
		Action:        permission.ActionRead,
		PathFromInput: pathInput,
		Schema:        objectSchema("path"),
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			path, err := cleanWorkspacePath(root, stringInput(call.Input, "path"))
			if err != nil {
				return tools.Result{}, err
			}
			data, err := os.ReadFile(path)
			if err != nil {
				return tools.Result{}, err
			}
			return tools.Result{Text: string(data)}, nil
		},
	})

	reg.Register(tools.Tool{
		Name:          "list_files",
		Description:   "List direct children of a workspace directory.",
		ReadOnly:      true,
		Action:        permission.ActionRead,
		PathFromInput: pathInput,
		Schema:        objectSchema("path"),
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			path, err := cleanWorkspacePath(root, stringInputDefault(call.Input, "path", "."))
			if err != nil {
				return tools.Result{}, err
			}
			entries, err := os.ReadDir(path)
			if err != nil {
				return tools.Result{}, err
			}
			names := make([]string, 0, len(entries))
			for _, entry := range entries {
				name := entry.Name()
				if entry.IsDir() {
					name += "/"
				}
				names = append(names, name)
			}
			sort.Strings(names)
			return tools.Result{Text: strings.Join(names, "\n")}, nil
		},
	})

	reg.Register(tools.Tool{
		Name:        "search_files",
		Description: "Search workspace text files for a literal query.",
		ReadOnly:    true,
		Action:      permission.ActionRead,
		Schema:      objectSchema("query"),
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			query := stringInput(call.Input, "query")
			if query == "" {
				return tools.Result{}, errors.New("query is required")
			}
			var matches []string
			ignored := newGitIgnoreChecker(root)
			err := filepath.WalkDir(root, func(path string, entry os.DirEntry, err error) error {
				if err != nil {
					return err
				}
				select {
				case <-ctx.Done():
					return ctx.Err()
				default:
				}
				if entry.IsDir() {
					if strings.HasPrefix(entry.Name(), ".git") {
						return filepath.SkipDir
					}
					if ignored(path, true) {
						return filepath.SkipDir
					}
					return nil
				}
				if ignored(path, false) {
					return nil
				}
				file, err := os.Open(path)
				if err != nil {
					return nil
				}
				defer file.Close()
				text, err := looksLikeText(file)
				if err != nil || !text {
					return nil
				}
				rel, _ := filepath.Rel(root, path)
				scanner := bufio.NewScanner(file)
				lineNo := 0
				for scanner.Scan() {
					lineNo++
					line := scanner.Text()
					if strings.Contains(line, query) {
						matches = append(matches, fmt.Sprintf("%s:%d:%s", rel, lineNo, line))
					}
				}
				return nil
			})
			if err != nil {
				return tools.Result{}, err
			}
			return tools.Result{Text: strings.Join(matches, "\n")}, nil
		},
	})
}

func newGitIgnoreChecker(root string) func(string, bool) bool {
	rootAbs, err := filepath.Abs(root)
	if err != nil {
		return func(string, bool) bool { return false }
	}
	if err := exec.Command("git", "-C", rootAbs, "rev-parse", "--is-inside-work-tree").Run(); err != nil {
		return func(string, bool) bool { return false }
	}
	return func(path string, isDir bool) bool {
		rel, err := filepath.Rel(rootAbs, path)
		if err != nil || rel == "." {
			return false
		}
		rel = filepath.ToSlash(rel)
		if isDir && !strings.HasSuffix(rel, "/") {
			rel += "/"
		}
		err = exec.Command("git", "-C", rootAbs, "check-ignore", "-q", "--", rel).Run()
		return err == nil
	}
}

func looksLikeText(file *os.File) (bool, error) {
	const sniffSize = 8192
	buf := make([]byte, sniffSize)
	n, err := file.Read(buf)
	if err != nil && err != io.EOF {
		return false, err
	}
	if _, err := file.Seek(0, io.SeekStart); err != nil {
		return false, err
	}
	for _, b := range buf[:n] {
		if b == 0 {
			return false, nil
		}
	}
	return true, nil
}

func cleanWorkspacePath(root, input string) (string, error) {
	if input == "" {
		input = "."
	}
	rootAbs, err := filepath.Abs(root)
	if err != nil {
		return "", err
	}
	joined := filepath.Join(rootAbs, input)
	abs, err := filepath.Abs(joined)
	if err != nil {
		return "", err
	}
	rel, err := filepath.Rel(rootAbs, abs)
	if err != nil {
		return "", err
	}
	if rel == ".." || strings.HasPrefix(rel, ".."+string(filepath.Separator)) {
		return "", fmt.Errorf("path escapes workspace: %s", input)
	}
	return abs, nil
}
