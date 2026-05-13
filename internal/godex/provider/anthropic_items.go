package provider

import (
	"encoding/json"
	"fmt"
	"regexp"
	"strings"
)

var anthropicToolName = regexp.MustCompile(`^[a-zA-Z0-9_-]{1,64}$`)

type AnthropicInput struct {
	System      []AnthropicBlock   `json:"system"`
	Messages    []AnthropicMessage `json:"messages"`
	Tools       []AnthropicTool    `json:"tools"`
	DebugEvents []string           `json:"-"`
}

type AnthropicMessage struct {
	Role    string           `json:"role"`
	Content []AnthropicBlock `json:"content"`
}

type AnthropicBlock struct {
	Type      string          `json:"type"`
	Text      string          `json:"text,omitempty"`
	ID        string          `json:"id,omitempty"`
	Name      string          `json:"name,omitempty"`
	Input     json.RawMessage `json:"input,omitempty"`
	ToolUseID string          `json:"tool_use_id,omitempty"`
	Content   any             `json:"content,omitempty"`
	IsError   bool            `json:"is_error,omitempty"`
}

type AnthropicTool struct {
	Name        string         `json:"name"`
	Description string         `json:"description,omitempty"`
	InputSchema map[string]any `json:"input_schema"`
}

func AnthropicInputFromResponsesItems(items []Item, tools []ToolSpec) (AnthropicInput, error) {
	convertedTools, err := anthropicTools(tools)
	if err != nil {
		return AnthropicInput{}, err
	}
	input := AnthropicInput{
		System:   []AnthropicBlock{},
		Messages: []AnthropicMessage{},
		Tools:    convertedTools,
	}
	for _, item := range items {
		switch item.Kind {
		case ItemMessage:
			text := strings.TrimSpace(item.Text)
			if text == "" {
				continue
			}
			switch item.Role {
			case string(RoleSystem):
				input.System = append(input.System, AnthropicBlock{Type: "text", Text: text})
			case string(RoleAssistant):
				input.appendBlock("assistant", AnthropicBlock{Type: "text", Text: text})
			default:
				input.appendBlock("user", AnthropicBlock{Type: "text", Text: text})
			}
		case ItemFunctionCall:
			block := AnthropicBlock{
				Type:  "tool_use",
				ID:    item.ToolCallID,
				Name:  item.ToolName,
				Input: anthropicToolInput(item),
			}
			input.appendBlock("assistant", block)
		case ItemFunctionOut:
			block := AnthropicBlock{
				Type:      "tool_result",
				ToolUseID: item.ToolCallID,
				Content:   item.Text,
			}
			input.appendToolResult(block)
		case ItemReasoning:
			continue
		case ItemCompaction:
			text := strings.TrimSpace(item.Text)
			if text == "" {
				return AnthropicInput{}, NonPortableItemError{ItemID: item.ID, Kind: string(item.Kind), Provider: "anthropic", Reason: "OpenAI raw compaction output has no provider-neutral text"}
			}
			input.appendBlock("user", AnthropicBlock{Type: "text", Text: text})
		case ItemRaw:
			if item.ID != "" {
				input.DebugEvents = append(input.DebugEvents, "omitted raw item "+item.ID)
			}
			continue
		}
	}
	return input, nil
}

func (input *AnthropicInput) appendToolResult(block AnthropicBlock) {
	if len(input.Messages) > 0 {
		last := &input.Messages[len(input.Messages)-1]
		if last.Role == "user" && userMessageCanAcceptToolResult(*last) {
			last.Content = append(last.Content, block)
			return
		}
	}
	input.Messages = append(input.Messages, AnthropicMessage{Role: "user", Content: []AnthropicBlock{block}})
}

func userMessageCanAcceptToolResult(message AnthropicMessage) bool {
	for _, block := range message.Content {
		if block.Type != "tool_result" {
			return false
		}
	}
	return true
}

func (input *AnthropicInput) appendBlock(role string, block AnthropicBlock) {
	if role == "user" && block.Type == "text" && len(input.Messages) > 0 {
		last := &input.Messages[len(input.Messages)-1]
		if last.Role == "user" {
			last.Content = append(last.Content, block)
			return
		}
	}
	if role == "assistant" && len(input.Messages) > 0 {
		last := &input.Messages[len(input.Messages)-1]
		if last.Role == "assistant" {
			last.Content = append(last.Content, block)
			return
		}
	}
	input.Messages = append(input.Messages, AnthropicMessage{Role: role, Content: []AnthropicBlock{block}})
}

func anthropicTools(tools []ToolSpec) ([]AnthropicTool, error) {
	out := make([]AnthropicTool, 0, len(tools))
	for i, tool := range tools {
		name := strings.TrimSpace(tool.Name)
		if !anthropicToolName.MatchString(name) {
			return nil, fmt.Errorf("tools[%d].name %q is invalid for Anthropic: must match %s", i, tool.Name, anthropicToolName.String())
		}
		schema := tool.Schema
		if schema == nil {
			schema = map[string]any{"type": "object", "properties": map[string]any{}}
		}
		out = append(out, AnthropicTool{Name: name, Description: tool.Description, InputSchema: schema})
	}
	return out, nil
}

func anthropicToolInput(item Item) json.RawMessage {
	if len(item.RawJSON) > 0 {
		if input := rawObjectField(item.RawJSON, "input"); len(input) > 0 {
			return input
		}
		if arguments := rawObjectField(item.RawJSON, "arguments"); len(arguments) > 0 {
			var rawString string
			if json.Unmarshal(arguments, &rawString) == nil && json.Valid([]byte(rawString)) {
				return json.RawMessage(rawString)
			}
		}
		if json.Valid(item.RawJSON) {
			return append(json.RawMessage(nil), item.RawJSON...)
		}
	}
	if strings.TrimSpace(item.Text) != "" && json.Valid([]byte(item.Text)) {
		return json.RawMessage(item.Text)
	}
	return json.RawMessage(`{}`)
}

func rawObjectField(raw json.RawMessage, name string) json.RawMessage {
	var object map[string]json.RawMessage
	if json.Unmarshal(raw, &object) != nil {
		return nil
	}
	if value, ok := object[name]; ok {
		return append(json.RawMessage(nil), value...)
	}
	return nil
}
