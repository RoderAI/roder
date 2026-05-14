package builtin

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/pandelisz/gode/internal/godex/permission"
	"github.com/pandelisz/gode/internal/godex/tools"
	"github.com/pandelisz/gode/internal/godex/workspacepath"
)

func RegisterEditing(reg *tools.Registry, root string) {
	reg.Register(tools.Tool{
		Name:          "write_file",
		Description:   "Write a UTF-8 text file inside the workspace.",
		Action:        permission.ActionWrite,
		PathFromInput: pathInput,
		Schema:        objectSchema("path", "content"),
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			if err := ctx.Err(); err != nil {
				return tools.Result{}, err
			}
			path, err := workspacepath.CleanWorkspacePath(root, stringInput(call.Input, "path"))
			if err != nil {
				return tools.Result{}, err
			}
			if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
				return tools.Result{}, err
			}
			if err := os.WriteFile(path, []byte(stringInput(call.Input, "content")), 0o600); err != nil {
				return tools.Result{}, err
			}
			return tools.Result{Text: "wrote " + relPath(root, path)}, nil
		},
	})

	reg.Register(tools.Tool{
		Name:          "edit",
		Description:   "Replace an exact text range inside a workspace file.",
		Action:        permission.ActionWrite,
		PathFromInput: pathInput,
		Schema:        objectSchema("path", "old_string", "new_string"),
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			if err := ctx.Err(); err != nil {
				return tools.Result{}, err
			}
			path, err := workspacepath.CleanWorkspacePath(root, stringInput(call.Input, "path"))
			if err != nil {
				return tools.Result{}, err
			}
			oldText := stringInput(call.Input, "old_string")
			newText := stringInput(call.Input, "new_string")
			if oldText == "" {
				return tools.Result{}, errors.New("old_string is required")
			}
			data, err := os.ReadFile(path)
			if err != nil {
				return tools.Result{}, err
			}
			text := string(data)
			if !strings.Contains(text, oldText) {
				return tools.Result{}, errors.New("old_string does not match file")
			}
			updated := strings.Replace(text, oldText, newText, 1)
			if err := os.WriteFile(path, []byte(updated), 0o600); err != nil {
				return tools.Result{}, err
			}
			return tools.Result{Text: "edited " + relPath(root, path)}, nil
		},
	})

	reg.Register(tools.Tool{
		Name:          "multi_edit",
		Description:   "Apply multiple exact text replacements to one workspace file.",
		Action:        permission.ActionWrite,
		PathFromInput: pathInput,
		Schema:        objectSchema("path"),
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			if err := ctx.Err(); err != nil {
				return tools.Result{}, err
			}
			path, err := workspacepath.CleanWorkspacePath(root, stringInput(call.Input, "path"))
			if err != nil {
				return tools.Result{}, err
			}
			edits := arrayInput(call.Input, "edits")
			if len(edits) == 0 {
				return tools.Result{}, errors.New("edits are required")
			}
			data, err := os.ReadFile(path)
			if err != nil {
				return tools.Result{}, err
			}
			text := string(data)
			for index, raw := range edits {
				edit, ok := raw.(map[string]any)
				if !ok {
					return tools.Result{}, fmt.Errorf("edit %d must be an object", index)
				}
				oldText := stringInput(edit, "old_string")
				newText := stringInput(edit, "new_string")
				if oldText == "" {
					return tools.Result{}, fmt.Errorf("edit %d old_string is required", index)
				}
				if !strings.Contains(text, oldText) {
					return tools.Result{}, fmt.Errorf("edit %d old_string does not match file", index)
				}
				text = strings.Replace(text, oldText, newText, 1)
			}
			if err := os.WriteFile(path, []byte(text), 0o600); err != nil {
				return tools.Result{}, err
			}
			return tools.Result{Text: fmt.Sprintf("edited %s (%d replacements)", relPath(root, path), len(edits))}, nil
		},
	})
}

func pathInput(input map[string]any) string {
	return stringInput(input, "path")
}

func relPath(root string, path string) string {
	rel, err := filepath.Rel(root, path)
	if err != nil {
		return path
	}
	return filepath.ToSlash(rel)
}
