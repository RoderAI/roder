package appserver

import (
	"encoding/json"
	"fmt"
	"strings"

	"github.com/pandelisz/gode/internal/godex/imageinput"
	"github.com/pandelisz/gode/internal/godex/provider"
)

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
