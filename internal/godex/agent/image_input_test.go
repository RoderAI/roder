package agent

import (
	"context"
	"testing"

	"github.com/pandelisz/gode/internal/godex/provider"
	"github.com/pandelisz/gode/internal/godex/session"
)

func TestInitialContextReplacePromptUsesMultimodalInputItem(t *testing.T) {
	runner := NewRunner(Config{})
	image := provider.Image{URL: "data:image/png;base64,abc", Detail: "high"}
	ctx, err := runner.initialContext(context.Background(), RunRequest{
		Prompt:        "describe",
		InputItems:    []provider.Item{{Kind: provider.ItemMessage, Role: "user", Text: "describe", Images: []provider.Image{image}}},
		ReplacePrompt: true,
	}, nil, "describe")
	if err != nil {
		t.Fatalf("initial context: %v", err)
	}
	if len(ctx.Messages) != 1 {
		t.Fatalf("messages = %#v", ctx.Messages)
	}
	if ctx.Messages[0].Content != "describe" || len(ctx.Messages[0].Images) != 1 {
		t.Fatalf("message = %#v", ctx.Messages[0])
	}
	if len(ctx.InputItems) != 1 || len(ctx.InputItems[0].Images) != 1 {
		t.Fatalf("items = %#v", ctx.InputItems)
	}
}

func TestSessionItemsPreserveImages(t *testing.T) {
	items := providerItemsFromSessionItems([]session.Item{{
		ID:     "u1",
		Kind:   session.ItemMessage,
		Role:   "user",
		Text:   "look",
		Images: []session.Image{{URL: "data:image/png;base64,abc", Detail: "high"}},
	}})
	if len(items) != 1 || len(items[0].Images) != 1 {
		t.Fatalf("provider items = %#v", items)
	}
	messages := providerMessagesFromSessionItems([]session.Item{{
		ID:     "u1",
		Kind:   session.ItemMessage,
		Role:   "user",
		Text:   "look",
		Images: []session.Image{{URL: "data:image/png;base64,abc", Detail: "high"}},
	}})
	if len(messages) != 1 || len(messages[0].Images) != 1 {
		t.Fatalf("provider messages = %#v", messages)
	}
}
