package attachments

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

type Kind string

const (
	KindText  Kind = "text"
	KindImage Kind = "image"
)

type Attachment struct {
	Path string
	Kind Kind
}

func New(path string) Attachment {
	kind := KindText
	switch strings.ToLower(filepath.Ext(path)) {
	case ".png", ".jpg", ".jpeg", ".gif", ".webp":
		kind = KindImage
	}
	return Attachment{Path: filepath.ToSlash(strings.TrimSpace(path)), Kind: kind}
}

func AppendTextContext(workspace string, prompt string, attachments []Attachment) (string, error) {
	if len(attachments) == 0 {
		return prompt, nil
	}
	var blocks []string
	for _, attachment := range attachments {
		if attachment.Kind == KindImage {
			return "", fmt.Errorf("image attachments are not supported yet: %s", attachment.Path)
		}
		path, err := cleanWorkspacePath(workspace, attachment.Path)
		if err != nil {
			return "", err
		}
		data, err := os.ReadFile(path)
		if err != nil {
			return "", err
		}
		blocks = append(blocks, fmt.Sprintf("File %s:\n```text\n%s\n```", attachment.Path, strings.TrimRight(string(data), "\n")))
	}
	return strings.TrimSpace(prompt) + "\n\nAttached context:\n\n" + strings.Join(blocks, "\n\n"), nil
}

func cleanWorkspacePath(workspace string, rel string) (string, error) {
	root, err := filepath.Abs(workspace)
	if err != nil {
		return "", err
	}
	path, err := filepath.Abs(filepath.Join(root, rel))
	if err != nil {
		return "", err
	}
	if path != root && !strings.HasPrefix(path, root+string(os.PathSeparator)) {
		return "", fmt.Errorf("path escapes workspace: %s", rel)
	}
	return path, nil
}
