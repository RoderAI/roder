package provider

import (
	"context"
	"encoding/json"
	"fmt"

	anthropic "github.com/anthropics/anthropic-sdk-go"
	"github.com/anthropics/anthropic-sdk-go/option"
)

const defaultAnthropicMaxTokens int64 = 4096

type AnthropicConfig struct {
	Model     string
	MaxTokens int64
	BaseURL   string
	APIKey    string
}

type Anthropic struct {
	client    anthropic.Client
	model     string
	maxTokens int64
}

func NewAnthropic(model string, opts ...option.RequestOption) *Anthropic {
	return NewAnthropicWithConfig(AnthropicConfig{Model: model}, opts...)
}

func NewAnthropicWithConfig(cfg AnthropicConfig, opts ...option.RequestOption) *Anthropic {
	if cfg.Model == "" {
		cfg.Model = "claude-sonnet-4-6"
	}
	if cfg.MaxTokens == 0 {
		cfg.MaxTokens = defaultAnthropicMaxTokens
	}
	if cfg.BaseURL != "" {
		opts = append([]option.RequestOption{option.WithBaseURL(cfg.BaseURL)}, opts...)
	}
	if cfg.APIKey != "" {
		opts = append([]option.RequestOption{option.WithAPIKey(cfg.APIKey)}, opts...)
	}
	return &Anthropic{
		client:    anthropic.NewClient(opts...),
		model:     cfg.Model,
		maxTokens: cfg.MaxTokens,
	}
}

func (a *Anthropic) Name() string {
	return "anthropic"
}

func (a *Anthropic) Stream(ctx context.Context, req Request) (<-chan Event, <-chan error) {
	events := make(chan Event)
	errs := make(chan error, 1)
	go func() {
		defer close(events)
		defer close(errs)

		params, err := a.messageParams(req)
		if err != nil {
			errs <- err
			return
		}
		stream := a.client.Messages.NewStreaming(ctx, params)
		defer stream.Close()

		state := newAnthropicStreamState()
		for stream.Next() {
			emitted, err := state.Handle(stream.Current())
			if err != nil {
				errs <- err
				return
			}
			for _, ev := range emitted {
				select {
				case <-ctx.Done():
					errs <- ctx.Err()
					return
				case events <- ev:
				}
			}
		}
		if err := stream.Err(); err != nil {
			errs <- &ProviderError{Message: "Anthropic stream request failed: " + err.Error()}
			return
		}
		events <- state.CompletedEvent()
	}()
	return events, errs
}

func (a *Anthropic) messageParams(req Request) (anthropic.MessageNewParams, error) {
	items := req.InputItems
	if len(items) == 0 {
		items = itemsFromMessages(req.Messages)
	}
	input, err := AnthropicInputFromResponsesItems(items, req.Tools)
	if err != nil {
		return anthropic.MessageNewParams{}, err
	}
	return anthropic.MessageNewParams{
		Model:     anthropic.Model(a.model),
		MaxTokens: a.maxTokens,
		System:    anthropicSystemBlocks(input.System),
		Messages:  anthropicMessages(input.Messages),
		Tools:     anthropicSDKTools(input.Tools),
	}, nil
}

func itemsFromMessages(messages []Message) []Item {
	items := make([]Item, 0, len(messages))
	for i, msg := range messages {
		id := fmt.Sprintf("message_%d", i)
		if len(msg.RawJSON) > 0 {
			items = append(items, Item{ID: id, Kind: ItemRaw, RawJSON: append([]byte(nil), msg.RawJSON...)})
			continue
		}
		if msg.Role == RoleAssistant && msg.ToolCallID != "" && msg.ToolName != "" {
			items = append(items, Item{ID: id, Kind: ItemFunctionCall, ToolName: msg.ToolName, ToolCallID: msg.ToolCallID, Text: msg.ToolArguments})
			continue
		}
		if msg.Role == RoleTool && msg.ToolCallID != "" {
			items = append(items, Item{ID: id, Kind: ItemFunctionOut, ToolCallID: msg.ToolCallID, Text: msg.Content})
			continue
		}
		items = append(items, Item{ID: id, Kind: ItemMessage, Role: string(msg.Role), Text: msg.Content})
	}
	return items
}

func anthropicSystemBlocks(blocks []AnthropicBlock) []anthropic.TextBlockParam {
	out := make([]anthropic.TextBlockParam, 0, len(blocks))
	for _, block := range blocks {
		if block.Type == "text" && block.Text != "" {
			out = append(out, anthropic.TextBlockParam{Text: block.Text})
		}
	}
	return out
}

func anthropicMessages(messages []AnthropicMessage) []anthropic.MessageParam {
	out := make([]anthropic.MessageParam, 0, len(messages))
	for _, message := range messages {
		blocks := anthropicContentBlocks(message.Content)
		if message.Role == "assistant" {
			out = append(out, anthropic.NewAssistantMessage(blocks...))
		} else {
			out = append(out, anthropic.NewUserMessage(blocks...))
		}
	}
	return out
}

func anthropicContentBlocks(blocks []AnthropicBlock) []anthropic.ContentBlockParamUnion {
	out := make([]anthropic.ContentBlockParamUnion, 0, len(blocks))
	for _, block := range blocks {
		switch block.Type {
		case "text":
			out = append(out, anthropic.ContentBlockParamUnion{OfText: &anthropic.TextBlockParam{Text: block.Text}})
		case "tool_use":
			var input any = map[string]any{}
			if len(block.Input) > 0 {
				_ = json.Unmarshal(block.Input, &input)
			}
			out = append(out, anthropic.NewToolUseBlock(block.ID, input, block.Name))
		case "tool_result":
			out = append(out, anthropic.NewToolResultBlock(block.ToolUseID, blockContentString(block.Content), block.IsError))
		}
	}
	return out
}

func blockContentString(value any) string {
	switch value := value.(type) {
	case string:
		return value
	case nil:
		return ""
	default:
		data, err := json.Marshal(value)
		if err != nil {
			return fmt.Sprint(value)
		}
		return string(data)
	}
}

func anthropicSDKTools(tools []AnthropicTool) []anthropic.ToolUnionParam {
	out := make([]anthropic.ToolUnionParam, 0, len(tools))
	for _, tool := range tools {
		param := anthropic.ToolParam{
			Name:        tool.Name,
			InputSchema: anthropicToolInputSchema(tool.InputSchema),
		}
		if tool.Description != "" {
			param.Description = anthropic.String(tool.Description)
		}
		out = append(out, anthropic.ToolUnionParam{OfTool: &param})
	}
	return out
}

func anthropicToolInputSchema(schema map[string]any) anthropic.ToolInputSchemaParam {
	out := anthropic.ToolInputSchemaParam{ExtraFields: map[string]any{}}
	for key, value := range schema {
		switch key {
		case "properties":
			out.Properties = value
		case "required":
			out.Required = stringSlice(value)
		case "type":
			continue
		default:
			out.ExtraFields[key] = value
		}
	}
	return out
}

func stringSlice(value any) []string {
	switch value := value.(type) {
	case []string:
		return value
	case []any:
		out := make([]string, 0, len(value))
		for _, item := range value {
			if text, ok := item.(string); ok {
				out = append(out, text)
			}
		}
		return out
	default:
		return nil
	}
}
