package builtin

import (
	"bufio"
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/pandelisz/gode/internal/godex/tools"
)

func RegisterFilesystem(reg *tools.Registry, root string) {
	reg.Register(tools.Tool{
		Name:        "read_file",
		Description: "Read a UTF-8 text file inside the workspace.",
		ReadOnly:    true,
		Schema:      objectSchema("path"),
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
		Name:        "list_files",
		Description: "List direct children of a workspace directory.",
		ReadOnly:    true,
		Schema:      objectSchema("path"),
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
		Schema:      objectSchema("query"),
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			query := stringInput(call.Input, "query")
			if query == "" {
				return tools.Result{}, errors.New("query is required")
			}
			var matches []string
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
					return nil
				}
				file, err := os.Open(path)
				if err != nil {
					return nil
				}
				defer file.Close()
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
