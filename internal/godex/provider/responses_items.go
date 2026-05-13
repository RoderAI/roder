package provider

import (
	"encoding/json"
	"strings"

	"github.com/openai/openai-go/v3/packages/param"
	"github.com/openai/openai-go/v3/responses"
)

func responseInputItems(messages []Message) responses.ResponseInputParam {
	items := make(responses.ResponseInputParam, 0, len(messages))
	for _, msg := range messages {
		if len(msg.RawJSON) > 0 {
			items = append(items, param.Override[responses.ResponseInputItemUnionParam](json.RawMessage(msg.RawJSON)))
			continue
		}
		if msg.Role == RoleAssistant && msg.ToolCallID != "" && msg.ToolName != "" {
			arguments := strings.TrimSpace(msg.ToolArguments)
			if arguments == "" {
				arguments = "{}"
			}
			items = append(items, responses.ResponseInputItemParamOfFunctionCall(arguments, msg.ToolCallID, msg.ToolName))
			continue
		}
		if msg.Role == RoleTool && msg.ToolCallID != "" {
			items = append(items, responses.ResponseInputItemParamOfFunctionCallOutput(msg.ToolCallID, strings.TrimSpace(msg.Content)))
			continue
		}
		content := strings.TrimSpace(msg.Content)
		if content == "" {
			continue
		}
		items = append(items, responses.ResponseInputItemParamOfMessage(content, easyInputRole(msg.Role)))
	}
	return items
}

func rawResponseOutputItems(items []responses.ResponseOutputItemUnion) []json.RawMessage {
	out := make([]json.RawMessage, 0, len(items))
	for _, item := range items {
		if item.AsAny() != nil {
			if data, err := json.Marshal(item.AsAny()); err == nil {
				out = append(out, data)
				continue
			}
		}
		if data, err := json.Marshal(item); err == nil {
			out = append(out, data)
		}
	}
	return out
}

func easyInputRole(role Role) responses.EasyInputMessageRole {
	switch role {
	case RoleSystem:
		return responses.EasyInputMessageRoleSystem
	case RoleAssistant:
		return responses.EasyInputMessageRoleAssistant
	default:
		return responses.EasyInputMessageRoleUser
	}
}
