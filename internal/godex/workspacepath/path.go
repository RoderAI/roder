package workspacepath

import (
	"fmt"
	"path/filepath"
	"strings"
)

func CleanWorkspacePath(root, input string) (string, error) {
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

func CleanReadPath(root, input string) (string, error) {
	if input == "" {
		input = "."
	}
	var path string
	if filepath.IsAbs(input) {
		path = filepath.Clean(input)
	} else {
		rootAbs, err := filepath.Abs(root)
		if err != nil {
			return "", err
		}
		path = filepath.Join(rootAbs, input)
	}
	abs, err := filepath.Abs(path)
	if err != nil {
		return "", err
	}
	return abs, nil
}
