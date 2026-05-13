package builtin

import (
	"bufio"
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"

	"github.com/pandelisz/gode/internal/godex/permission"
	"github.com/pandelisz/gode/internal/godex/tools"
)

func RegisterSearch(reg *tools.Registry, root string) {
	reg.Register(tools.Tool{
		Name:        "grep",
		Description: "Search workspace text files for a literal query.",
		ReadOnly:    true,
		Action:      permission.ActionRead,
		Schema:      objectSchema("query"),
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			query := stringInput(call.Input, "query")
			if query == "" {
				return tools.Result{}, errors.New("query is required")
			}
			path, err := cleanWorkspacePath(root, stringInputDefault(call.Input, "path", "."))
			if err != nil {
				return tools.Result{}, err
			}
			if text, ok := runRG(ctx, root, path, query); ok {
				return tools.Result{Text: text}, nil
			}
			matches, err := grepFallback(ctx, root, path, query)
			if err != nil {
				return tools.Result{}, err
			}
			return tools.Result{Text: strings.Join(matches, "\n")}, nil
		},
	})

	reg.Register(tools.Tool{
		Name:        "glob",
		Description: "Find workspace files matching a glob pattern.",
		ReadOnly:    true,
		Action:      permission.ActionRead,
		Schema:      objectSchema("pattern"),
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			pattern := stringInput(call.Input, "pattern")
			if pattern == "" {
				return tools.Result{}, errors.New("pattern is required")
			}
			matches, err := globWorkspace(ctx, root, pattern)
			if err != nil {
				return tools.Result{}, err
			}
			return tools.Result{Text: strings.Join(matches, "\n")}, nil
		},
	})
}

func runRG(ctx context.Context, root string, path string, query string) (string, bool) {
	if _, err := exec.LookPath("rg"); err != nil {
		return "", false
	}
	cmd := exec.CommandContext(ctx, "rg", "--line-number", "--fixed-strings", "--color", "never", query, path)
	cmd.Dir = root
	out, err := cmd.CombinedOutput()
	if err != nil {
		if exitErr, ok := err.(*exec.ExitError); ok && exitErr.ExitCode() == 1 {
			return "", true
		}
		return "", false
	}
	return strings.TrimRight(string(out), "\n"), true
}

func grepFallback(ctx context.Context, root string, start string, query string) ([]string, error) {
	var matches []string
	ignored := newGitIgnoreChecker(root)
	err := filepath.WalkDir(start, func(path string, entry os.DirEntry, err error) error {
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
				matches = append(matches, fmt.Sprintf("%s:%d:%s", filepath.ToSlash(rel), lineNo, line))
			}
		}
		return nil
	})
	return matches, err
}

func globWorkspace(ctx context.Context, root string, pattern string) ([]string, error) {
	rootAbs, err := filepath.Abs(root)
	if err != nil {
		return nil, err
	}
	ignored := newGitIgnoreChecker(root)
	var matches []string
	err = filepath.WalkDir(rootAbs, func(path string, entry os.DirEntry, err error) error {
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
		rel, err := filepath.Rel(rootAbs, path)
		if err != nil {
			return err
		}
		rel = filepath.ToSlash(rel)
		ok, err := filepath.Match(pattern, rel)
		if err != nil {
			return err
		}
		if ok {
			matches = append(matches, rel)
		}
		return nil
	})
	sort.Strings(matches)
	return matches, err
}
