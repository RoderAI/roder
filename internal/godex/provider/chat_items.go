package provider

import (
	"encoding/json"
	"strings"
)

type ChatInput struct {
	Messages    []ChatMessage `json:"messages"`
	Tools       []ChatTool    `json:"tools,omitempty"`
	DebugEvents []string      `json:"-"`
}

type ChatMessage struct {
	Role       string         `json:"role"`
	Content    string         `json:"content,omitempty"`
	ToolCalls  []ChatToolCall `json:"tool_calls,omitempty"`
	ToolCallID string         `json:"tool_call_id,omitempty"`
}

type ChatToolCall struct {
	ID       string           `json:"id"`
	Type     string           `json:"type"`
	Function ChatFunctionCall `json:"function"`
}

type ChatFunctionCall struct {
	Name      string `json:"name"`
	Arguments string `json:"arguments"`
}

type ChatTool struct {
	Type     string       `json:"type"`
	Function ChatFunction `json:"function"`
}

type ChatFunction struct {
	Name        string         `json:"name"`
	Description string         `json:"description,omitempty"`
	Parameters  map[string]any `json:"parameters"`
}

func ChatInputFromResponsesItems(items []Item, tools []ToolSpec) (ChatInput, error) {
	input := ChatInput{Tools: ChatToolsFromToolSpecs(tools)}
	for _, item := range items {
		switch item.Kind {
		case ItemMessage:
			text := strings.TrimSpace(item.Text)
			if text == "" {
				continue
			}
			role := item.Role
			if role == "" {
				role = string(RoleUser)
			}
			input.Messages = append(input.Messages, ChatMessage{Role: role, Content: text})
		case ItemFunctionCall:
			input.appendToolCall(ChatToolCall{
				ID:   firstChatValue(item.ToolCallID, item.ID),
				Type: "function",
				Function: ChatFunctionCall{
					Name:      item.ToolName,
					Arguments: chatToolArguments(item),
				},
			})
		case ItemFunctionOut:
			input.Messages = append(input.Messages, ChatMessage{
				Role:       string(RoleTool),
				ToolCallID: item.ToolCallID,
				Content:    item.Text,
			})
		case ItemReasoning:
			continue
		case ItemCompaction:
			text := strings.TrimSpace(item.Text)
			if text == "" {
				return ChatInput{}, NonPortableItemError{ItemID: item.ID, Kind: string(item.Kind), Provider: "chat_completions", Reason: "provider-specific compaction is nonportable; provider-neutral compaction text is required"}
			}
			input.Messages = append(input.Messages, ChatMessage{Role: string(RoleUser), Content: text})
		case ItemRaw:
			if item.ID != "" {
				input.DebugEvents = append(input.DebugEvents, "omitted raw item "+item.ID)
			}
			continue
		}
	}
	return input, nil
}

func (input *ChatInput) appendToolCall(call ChatToolCall) {
	if len(input.Messages) > 0 {
		last := &input.Messages[len(input.Messages)-1]
		if last.Role == string(RoleAssistant) {
			last.ToolCalls = append(last.ToolCalls, call)
			return
		}
	}
	input.Messages = append(input.Messages, ChatMessage{
		Role:      string(RoleAssistant),
		ToolCalls: []ChatToolCall{call},
	})
}

func ChatToolsFromToolSpecs(tools []ToolSpec) []ChatTool {
	out := make([]ChatTool, 0, len(tools))
	for _, tool := range tools {
		schema := tool.Schema
		if schema == nil {
			schema = map[string]any{"type": "object", "properties": map[string]any{}}
		}
		out = append(out, ChatTool{
			Type: "function",
			Function: ChatFunction{
				Name:        strings.TrimSpace(tool.Name),
				Description: tool.Description,
				Parameters:  schema,
			},
		})
	}
	return out
}

func chatToolArguments(item Item) string {
	if text := strings.TrimSpace(item.Text); text != "" && json.Valid([]byte(text)) {
		return text
	}
	if len(item.RawJSON) > 0 {
		if input := rawObjectField(item.RawJSON, "input"); len(input) > 0 {
			return string(input)
		}
		if arguments := rawObjectField(item.RawJSON, "arguments"); len(arguments) > 0 {
			var rawString string
			if json.Unmarshal(arguments, &rawString) == nil {
				return rawString
			}
			return string(arguments)
		}
	}
	return `{}`
}

func firstChatValue(values ...string) string {
	for _, value := range values {
		if strings.TrimSpace(value) != "" {
			return strings.TrimSpace(value)
		}
	}
	return ""
}
