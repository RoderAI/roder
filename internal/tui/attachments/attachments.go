package attachments

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/pandelisz/gode/internal/godex/imageinput"
	"github.com/pandelisz/gode/internal/godex/provider"
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

type PromptInput struct {
	Prompt        string
	Items         []provider.Item
	ReplacePrompt bool
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

func BuildPromptInput(workspace string, prompt string, attachments []Attachment) (PromptInput, error) {
	textPrompt, err := AppendTextContext(workspace, prompt, textAttachments(attachments))
	if err != nil {
		return PromptInput{}, err
	}
	images, err := promptImages(workspace, attachments)
	if err != nil {
		return PromptInput{}, err
	}
	if len(images) == 0 {
		return PromptInput{Prompt: textPrompt}, nil
	}
	return PromptInput{
		Prompt: textPrompt,
		Items: []provider.Item{{
			Kind:   provider.ItemMessage,
			Role:   string(provider.RoleUser),
			Text:   strings.TrimSpace(textPrompt),
			Images: images,
		}},
		ReplacePrompt: true,
	}, nil
}

func textAttachments(attachments []Attachment) []Attachment {
	out := make([]Attachment, 0, len(attachments))
	for _, attachment := range attachments {
		if attachment.Kind == KindText {
			out = append(out, attachment)
		}
	}
	return out
}

func promptImages(workspace string, attachments []Attachment) ([]provider.Image, error) {
	images := make([]provider.Image, 0, len(attachments))
	for _, attachment := range attachments {
		if attachment.Kind != KindImage {
			continue
		}
		path, err := cleanImagePath(workspace, attachment.Path)
		if err != nil {
			return nil, err
		}
		encoded, err := imageinput.EncodeFile(path)
		if err != nil {
			return nil, err
		}
		images = append(images, provider.Image{URL: encoded.URL, Detail: "high"})
	}
	return images, nil
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

func cleanImagePath(workspace string, path string) (string, error) {
	if filepath.IsAbs(path) {
		return filepath.Clean(path), nil
	}
	return cleanWorkspacePath(workspace, path)
}
