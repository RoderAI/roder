package builtin

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"

	"github.com/pandelisz/gode/internal/godex/permission"
	"github.com/pandelisz/gode/internal/godex/tools"
	"github.com/pandelisz/gode/internal/godex/workspacepath"
)

func RegisterPatch(reg *tools.Registry, root string) {
	reg.Register(tools.Tool{
		Name:        "apply_patch",
		Description: "Apply a unified patch in the workspace using git apply.",
		ReadOnly:    false,
		Action:      permission.ActionWrite,
		Schema:      objectSchema("patch"),
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			patch := stringInput(call.Input, "patch")
			if isCodexPatch(patch) {
				return applyCodexPatch(ctx, root, patch)
			}
			cmd := exec.CommandContext(ctx, "git", "apply", "--whitespace=nowarn", "-")
			cmd.Dir = root
			cmd.Stdin = strings.NewReader(patch)
			out, err := cmd.CombinedOutput()
			result := tools.Result{Text: strings.TrimSpace(string(out))}
			if err != nil {
				return result, fmt.Errorf("failed to apply patch: %w", err)
			}
			return result, nil
		},
	})
}

type codexPatchChange struct {
	op     string
	path   string
	moveTo string
	lines  []string
	hunks  []codexPatchHunk
}

type codexPatchHunk struct {
	oldLines []string
	newLines []string
}

func isCodexPatch(patch string) bool {
	return strings.HasPrefix(strings.TrimSpace(patch), "*** Begin Patch")
}

func applyCodexPatch(ctx context.Context, root, patch string) (tools.Result, error) {
	changes, err := parseCodexPatch(patch)
	if err != nil {
		return tools.Result{}, fmt.Errorf("failed to parse patch: %w", err)
	}
	var summaries []string
	for _, change := range changes {
		if err := ctx.Err(); err != nil {
			return tools.Result{}, err
		}
		summary, err := applyCodexPatchChange(root, change)
		if err != nil {
			return tools.Result{Text: err.Error()}, fmt.Errorf("failed to apply patch: %w", err)
		}
		summaries = append(summaries, summary)
	}
	if len(summaries) == 0 {
		return tools.Result{}, fmt.Errorf("failed to apply patch: no changes found")
	}
	return tools.Result{Text: "Success. " + strings.Join(summaries, "\n")}, nil
}

func parseCodexPatch(patch string) ([]codexPatchChange, error) {
	patch = strings.ReplaceAll(patch, "\r\n", "\n")
	lines := strings.Split(patch, "\n")
	for len(lines) > 0 && strings.TrimSpace(lines[len(lines)-1]) == "" {
		lines = lines[:len(lines)-1]
	}
	if len(lines) < 2 || strings.TrimSpace(lines[0]) != "*** Begin Patch" {
		return nil, fmt.Errorf("missing *** Begin Patch")
	}
	var changes []codexPatchChange
parseLoop:
	for i := 1; i < len(lines); {
		line := lines[i]
		switch {
		case line == "*** End Patch":
			return changes, nil
		case strings.HasPrefix(line, "*** Add File: "):
			change := codexPatchChange{op: "add", path: strings.TrimSpace(strings.TrimPrefix(line, "*** Add File: "))}
			i++
			for i < len(lines) && !strings.HasPrefix(lines[i], "*** ") {
				if !strings.HasPrefix(lines[i], "+") {
					return nil, fmt.Errorf("add file %s contains non-add line %q", change.path, lines[i])
				}
				change.lines = append(change.lines, strings.TrimPrefix(lines[i], "+"))
				i++
			}
			changes = append(changes, change)
		case strings.HasPrefix(line, "*** Delete File: "):
			changes = append(changes, codexPatchChange{op: "delete", path: strings.TrimSpace(strings.TrimPrefix(line, "*** Delete File: "))})
			i++
		case strings.HasPrefix(line, "*** Update File: "):
			change := codexPatchChange{op: "update", path: strings.TrimSpace(strings.TrimPrefix(line, "*** Update File: "))}
			i++
			for i < len(lines) {
				line = lines[i]
				switch {
				case line == "*** End Patch", strings.HasPrefix(line, "*** Add File: "), strings.HasPrefix(line, "*** Delete File: "), strings.HasPrefix(line, "*** Update File: "):
					changes = append(changes, change)
					continue parseLoop
				case strings.HasPrefix(line, "*** Move to: "):
					change.moveTo = strings.TrimSpace(strings.TrimPrefix(line, "*** Move to: "))
					i++
				case strings.HasPrefix(line, "@@"):
					hunk, next, err := parseCodexPatchHunk(lines, i+1)
					if err != nil {
						return nil, fmt.Errorf("%s: %w", change.path, err)
					}
					change.hunks = append(change.hunks, hunk)
					i = next
				default:
					return nil, fmt.Errorf("%s: expected hunk header, got %q", change.path, line)
				}
			}
			changes = append(changes, change)
		default:
			return nil, fmt.Errorf("unexpected patch line %q", line)
		}
	}
	return nil, fmt.Errorf("missing *** End Patch")
}

func parseCodexPatchHunk(lines []string, start int) (codexPatchHunk, int, error) {
	var hunk codexPatchHunk
	i := start
	for i < len(lines) {
		line := lines[i]
		if strings.HasPrefix(line, "@@") || strings.HasPrefix(line, "*** ") {
			break
		}
		if line == "*** End of File" {
			i++
			continue
		}
		if line == "" {
			return hunk, i, fmt.Errorf("empty hunk line must be prefixed with space, +, or -")
		}
		body := line[1:]
		switch line[0] {
		case ' ':
			hunk.oldLines = append(hunk.oldLines, body)
			hunk.newLines = append(hunk.newLines, body)
		case '-':
			hunk.oldLines = append(hunk.oldLines, body)
		case '+':
			hunk.newLines = append(hunk.newLines, body)
		default:
			return hunk, i, fmt.Errorf("invalid hunk line prefix %q", line[0])
		}
		i++
	}
	if len(hunk.oldLines) == 0 && len(hunk.newLines) == 0 {
		return hunk, i, fmt.Errorf("empty hunk")
	}
	return hunk, i, nil
}

func applyCodexPatchChange(root string, change codexPatchChange) (string, error) {
	path, err := workspacepath.CleanWorkspacePath(root, change.path)
	if err != nil {
		return "", err
	}
	switch change.op {
	case "add":
		if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
			return "", err
		}
		if _, err := os.Stat(path); err == nil {
			return "", fmt.Errorf("%s already exists", change.path)
		} else if !os.IsNotExist(err) {
			return "", err
		}
		if err := os.WriteFile(path, []byte(joinPatchLines(change.lines)), 0o600); err != nil {
			return "", err
		}
		return "Added " + relPath(root, path), nil
	case "delete":
		if err := os.Remove(path); err != nil {
			return "", err
		}
		return "Deleted " + relPath(root, path), nil
	case "update":
		updated, err := applyCodexPatchUpdate(path, change)
		if err != nil {
			return "", err
		}
		targetPath := path
		if change.moveTo != "" {
			targetPath, err = workspacepath.CleanWorkspacePath(root, change.moveTo)
			if err != nil {
				return "", err
			}
			if err := os.MkdirAll(filepath.Dir(targetPath), 0o700); err != nil {
				return "", err
			}
		}
		if err := os.WriteFile(targetPath, []byte(updated), 0o600); err != nil {
			return "", err
		}
		if targetPath != path {
			if err := os.Remove(path); err != nil {
				return "", err
			}
			return "Moved " + relPath(root, path) + " to " + relPath(root, targetPath), nil
		}
		return "Updated " + relPath(root, path), nil
	default:
		return "", fmt.Errorf("unsupported patch operation %q", change.op)
	}
}

func applyCodexPatchUpdate(path string, change codexPatchChange) (string, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return "", err
	}
	text := string(data)
	for _, hunk := range change.hunks {
		oldText := joinPatchLinesWithoutForcedTrailingNewline(hunk.oldLines)
		newText := joinPatchLinesWithoutForcedTrailingNewline(hunk.newLines)
		if oldText == "" {
			text = newText + text
			continue
		}
		if !strings.Contains(text, oldText) {
			return "", fmt.Errorf("expected hunk not found in %s:\n%s", change.path, oldText)
		}
		text = strings.Replace(text, oldText, newText, 1)
	}
	return text, nil
}

func joinPatchLines(lines []string) string {
	if len(lines) == 0 {
		return ""
	}
	return strings.Join(lines, "\n") + "\n"
}

func joinPatchLinesWithoutForcedTrailingNewline(lines []string) string {
	return strings.Join(lines, "\n")
}
