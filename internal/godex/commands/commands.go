package commands

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
)

var placeholderPattern = regexp.MustCompile(`\$([A-Z][A-Z0-9_]*)`)

type LoadOptions struct {
	Workspace string
	HomeDir   string
}

type Catalog struct {
	Commands []Command
}

type Command struct {
	ID           string
	Scope        string
	Path         string
	Prompt       string
	Placeholders []string
}

type ExpandResult struct {
	Prompt  string
	Command *Command
}

func Load(opts LoadOptions) (Catalog, error) {
	workspace := absOrDefault(opts.Workspace, ".")
	home := strings.TrimSpace(opts.HomeDir)
	if home == "" {
		home, _ = os.UserHomeDir()
	}
	roots := []commandRoot{
		{Scope: "user", Path: filepath.Join(home, ".config", "gode", "commands")},
		{Scope: "user", Path: filepath.Join(home, ".gode", "commands")},
		{Scope: "project", Path: filepath.Join(workspace, ".gode", "commands")},
	}
	seen := map[string]struct{}{}
	var catalog Catalog
	for _, root := range roots {
		commands, err := loadRoot(root)
		if err != nil {
			return Catalog{}, err
		}
		for _, command := range commands {
			if _, ok := seen[command.ID]; ok {
				continue
			}
			seen[command.ID] = struct{}{}
			catalog.Commands = append(catalog.Commands, command)
		}
	}
	return catalog, nil
}

func Expand(ctx context.Context, prompt string, catalog Catalog) (ExpandResult, error) {
	if err := ctx.Err(); err != nil {
		return ExpandResult{}, err
	}
	trimmed := strings.TrimLeft(prompt, " \t")
	if !strings.HasPrefix(trimmed, "/") {
		return ExpandResult{Prompt: prompt}, nil
	}
	fields := strings.Fields(trimmed)
	if len(fields) == 0 {
		return ExpandResult{Prompt: prompt}, nil
	}
	token := strings.TrimPrefix(fields[0], "/")
	if strings.Contains(token, "/") {
		return ExpandResult{Prompt: prompt}, nil
	}
	command, ok, err := findCommand(token, catalog)
	if err != nil {
		return ExpandResult{}, err
	}
	if !ok {
		return ExpandResult{Prompt: prompt}, nil
	}
	values := map[string]string{}
	var trailing []string
	for _, field := range fields[1:] {
		key, value, ok := strings.Cut(field, "=")
		if ok && placeholderPattern.MatchString("$"+key) {
			values[key] = value
			continue
		}
		trailing = append(trailing, field)
	}
	expanded := command.Prompt
	for _, placeholder := range command.Placeholders {
		value, ok := values[placeholder]
		if !ok {
			return ExpandResult{}, fmt.Errorf("command %s requires %s=<value>", command.ID, placeholder)
		}
		expanded = strings.ReplaceAll(expanded, "$"+placeholder, value)
	}
	if len(trailing) > 0 {
		expanded = strings.TrimSpace(expanded) + "\n\n" + strings.Join(trailing, " ")
	}
	commandCopy := command
	return ExpandResult{Prompt: strings.TrimSpace(expanded), Command: &commandCopy}, nil
}

type commandRoot struct {
	Scope string
	Path  string
}

func loadRoot(root commandRoot) ([]Command, error) {
	if strings.TrimSpace(root.Path) == "" {
		return nil, nil
	}
	if _, err := os.Stat(root.Path); os.IsNotExist(err) {
		return nil, nil
	}
	var commands []Command
	err := filepath.WalkDir(root.Path, func(path string, entry os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if entry.IsDir() {
			return nil
		}
		if strings.ToLower(filepath.Ext(path)) != ".md" {
			return nil
		}
		command, err := readCommand(root, path)
		if err != nil {
			return err
		}
		commands = append(commands, command)
		return nil
	})
	if err != nil {
		return nil, err
	}
	sort.Slice(commands, func(i, j int) bool {
		return commands[i].ID < commands[j].ID
	})
	return commands, nil
}

func readCommand(root commandRoot, path string) (Command, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return Command{}, fmt.Errorf("read command %s: %w", path, err)
	}
	rel, err := filepath.Rel(root.Path, path)
	if err != nil {
		return Command{}, err
	}
	idPath := strings.TrimSuffix(filepath.ToSlash(rel), ".md")
	idPath = strings.ReplaceAll(idPath, "/", ":")
	prompt := strings.TrimSpace(string(data))
	return Command{
		ID:           root.Scope + ":" + idPath,
		Scope:        root.Scope,
		Path:         path,
		Prompt:       prompt,
		Placeholders: placeholders(prompt),
	}, nil
}

func placeholders(prompt string) []string {
	matches := placeholderPattern.FindAllStringSubmatch(prompt, -1)
	seen := map[string]struct{}{}
	var out []string
	for _, match := range matches {
		if len(match) < 2 {
			continue
		}
		name := match[1]
		if _, ok := seen[name]; ok {
			continue
		}
		seen[name] = struct{}{}
		out = append(out, name)
	}
	sort.Strings(out)
	return out
}

func findCommand(token string, catalog Catalog) (Command, bool, error) {
	for _, command := range catalog.Commands {
		if command.ID == token {
			return command, true, nil
		}
	}
	var matches []Command
	for _, command := range catalog.Commands {
		if strings.HasSuffix(command.ID, ":"+token) {
			matches = append(matches, command)
		}
	}
	if len(matches) == 0 {
		return Command{}, false, nil
	}
	if len(matches) == 1 {
		return matches[0], true, nil
	}
	for _, command := range matches {
		if command.Scope == "project" {
			return command, true, nil
		}
	}
	return Command{}, false, fmt.Errorf("command %q is ambiguous", token)
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
