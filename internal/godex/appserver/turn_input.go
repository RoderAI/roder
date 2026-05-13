package appserver

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"unicode/utf8"

	"github.com/pandelisz/gode/internal/godex/imageinput"
	"github.com/pandelisz/gode/internal/godex/provider"
)

const maxLocalFileInputBytes = 512 * 1024

type builtTurnInput struct {
	Prompt        string
	InputItems    []provider.Item
	ReplacePrompt bool
}

func buildTurnInput(prompt string, input []json.RawMessage) (builtTurnInput, error) {
	if prompt != "" && len(input) == 0 {
		return builtTurnInput{Prompt: prompt}, nil
	}
	parts := make([]string, 0, len(input)+1)
	if strings.TrimSpace(prompt) != "" {
		parts = append(parts, prompt)
	}
	images := make([]provider.Image, 0)
	for _, raw := range input {
		var item struct {
			Type     string `json:"type"`
			Text     string `json:"text"`
			Path     string `json:"path"`
			URL      string `json:"url"`
			ImageURL string `json:"image_url"`
			Detail   string `json:"detail"`
		}
		if err := json.Unmarshal(raw, &item); err != nil {
			return builtTurnInput{}, err
		}
		switch item.Type {
		case "", "text":
			if item.Text != "" {
				parts = append(parts, item.Text)
			}
		case "image":
			url := firstTurnInputNonEmpty(item.ImageURL, item.URL)
			if url == "" {
				return builtTurnInput{}, fmt.Errorf("image input requires image_url or url")
			}
			images = append(images, provider.Image{URL: url, Detail: firstTurnInputNonEmpty(item.Detail, "high")})
		case "local_image", "localImage":
			if item.Path == "" {
				return builtTurnInput{}, fmt.Errorf("local image input requires path")
			}
			encoded, err := imageinput.EncodeFile(item.Path)
			if err != nil {
				return builtTurnInput{}, err
			}
			images = append(images, provider.Image{URL: encoded.URL, Detail: firstTurnInputNonEmpty(item.Detail, "high")})
		case "file", "local_file", "localFile":
			if item.Path == "" {
				return builtTurnInput{}, fmt.Errorf("local file input requires path")
			}
			fileInput, err := buildLocalFileInput(item.Path)
			if err != nil {
				return builtTurnInput{}, err
			}
			if fileInput.Image != nil {
				images = append(images, provider.Image{URL: fileInput.Image.URL, Detail: firstTurnInputNonEmpty(item.Detail, "high")})
				if fileInput.Text != "" {
					parts = append(parts, fileInput.Text)
				}
				continue
			}
			parts = append(parts, fileInput.Text)
		default:
			return builtTurnInput{}, fmt.Errorf("unsupported input type %q", item.Type)
		}
	}
	text := strings.Join(parts, "\n")
	if len(images) == 0 {
		return builtTurnInput{Prompt: text}, nil
	}
	return builtTurnInput{
		Prompt: text,
		InputItems: []provider.Item{{
			Kind:   provider.ItemMessage,
			Role:   string(provider.RoleUser),
			Text:   strings.TrimSpace(text),
			Images: images,
		}},
		ReplacePrompt: true,
	}, nil
}

func firstTurnInputNonEmpty(values ...string) string {
	for _, value := range values {
		if strings.TrimSpace(value) != "" {
			return value
		}
	}
	return ""
}

func turnInputPrompt(prompt string, input []json.RawMessage) (string, error) {
	built, err := buildTurnInput(prompt, input)
	if err != nil {
		return "", err
	}
	return built.Prompt, nil
}

type localFileInput struct {
	Text  string
	Image *imageinput.Image
}

func buildLocalFileInput(path string) (localFileInput, error) {
	if strings.TrimSpace(path) == "" {
		return localFileInput{}, fmt.Errorf("local file input requires path")
	}
	if encoded, err := imageinput.EncodeFile(path); err == nil {
		text := fmt.Sprintf("Attached image: %s", path)
		return localFileInput{Text: text, Image: &encoded}, nil
	}

	info, err := os.Stat(path)
	if err != nil {
		return localFileInput{}, err
	}
	if info.IsDir() {
		return localFileInput{}, fmt.Errorf("local file input must be a file: %s", path)
	}

	data, truncated, err := readLocalFilePrefix(path, maxLocalFileInputBytes)
	if err != nil {
		return localFileInput{}, err
	}
	mime := http.DetectContentType(data)
	name := filepath.Base(path)
	if isBinaryContent(data) {
		return localFileInput{Text: fmt.Sprintf("Attached file: %s\nPath: %s\nMIME: %s\nSize: %d bytes\nContent omitted because the file appears to be binary.", name, path, mime, info.Size())}, nil
	}

	truncation := ""
	if truncated {
		truncation = fmt.Sprintf("\n\n[File truncated after %d bytes; original size %d bytes.]", maxLocalFileInputBytes, info.Size())
	}
	text := fmt.Sprintf("Attached file: %s\nPath: %s\nMIME: %s\nSize: %d bytes\n\n```%s\n%s%s\n```", name, path, mime, info.Size(), codeFenceLanguage(name), string(data), truncation)
	return localFileInput{Text: text}, nil
}

func readLocalFilePrefix(path string, limit int64) ([]byte, bool, error) {
	file, err := os.Open(path)
	if err != nil {
		return nil, false, err
	}
	defer file.Close()

	data := make([]byte, limit+1)
	n, err := io.ReadFull(file, data)
	if err != nil && err != io.ErrUnexpectedEOF && err != io.EOF {
		return nil, false, err
	}
	truncated := int64(n) > limit
	if truncated {
		n = int(limit)
	}
	return data[:n], truncated, nil
}

func isBinaryContent(data []byte) bool {
	if len(data) == 0 {
		return false
	}
	if !utf8.Valid(data) {
		return true
	}
	for _, b := range data {
		if b == 0 {
			return true
		}
	}
	return false
}

func codeFenceLanguage(name string) string {
	ext := strings.TrimPrefix(strings.ToLower(filepath.Ext(name)), ".")
	switch ext {
	case "go", "ts", "tsx", "js", "jsx", "json", "md", "py", "rs", "toml", "yaml", "yml", "sh", "css", "html":
		return ext
	default:
		return ""
	}
}
