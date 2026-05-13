package memory

import (
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"path/filepath"
	"strings"
)

type Scope struct {
	WorkspaceRoot string
	WorkspaceID   string
	DatabasePath  string
}

func NewScope(workspace string, databasePath string, dataDir string) (Scope, error) {
	root, err := NormalizeWorkspaceRoot(workspace)
	if err != nil {
		return Scope{}, err
	}
	dbPath := strings.TrimSpace(databasePath)
	if dbPath == "" {
		dbPath = defaultDatabasePath(dataDir)
	}
	if !filepath.IsAbs(dbPath) {
		if abs, err := filepath.Abs(dbPath); err == nil {
			dbPath = abs
		} else {
			dbPath = filepath.Clean(dbPath)
		}
	}
	sum := sha256.Sum256([]byte(root))
	return Scope{
		WorkspaceRoot: root,
		WorkspaceID:   "ws_" + hex.EncodeToString(sum[:])[:24],
		DatabasePath:  dbPath,
	}, nil
}

func NormalizeWorkspaceRoot(workspace string) (string, error) {
	workspace = strings.TrimSpace(workspace)
	if workspace == "" {
		return "", errors.New("workspace is required")
	}
	abs, err := filepath.Abs(workspace)
	if err != nil {
		return "", err
	}
	clean := filepath.Clean(abs)
	parts := splitPath(clean)
	for i, part := range parts {
		if part != ".git" {
			continue
		}
		if i+2 < len(parts) && parts[i+1] == "worktrees" {
			return joinPath(parts[:i]), nil
		}
		if i == len(parts)-1 {
			return joinPath(parts[:i]), nil
		}
	}
	return clean, nil
}

func splitPath(path string) []string {
	volume := filepath.VolumeName(path)
	rest := strings.TrimPrefix(path, volume)
	rest = strings.Trim(rest, string(filepath.Separator))
	if rest == "" {
		if volume != "" {
			return []string{volume + string(filepath.Separator)}
		}
		return []string{string(filepath.Separator)}
	}
	parts := strings.Split(rest, string(filepath.Separator))
	if filepath.IsAbs(path) {
		prefix := string(filepath.Separator)
		if volume != "" {
			prefix = volume + string(filepath.Separator)
		}
		return append([]string{prefix}, parts...)
	}
	return parts
}

func joinPath(parts []string) string {
	if len(parts) == 0 {
		return string(filepath.Separator)
	}
	if len(parts) == 1 && strings.HasSuffix(parts[0], string(filepath.Separator)) {
		return filepath.Clean(parts[0])
	}
	return filepath.Clean(filepath.Join(parts...))
}
