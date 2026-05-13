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
		if data, ok := rawResponseOutputItem(item); ok {
			out = append(out, data)
		}
	}
	return out
}

func rawResponseOutputItem(item responses.ResponseOutputItemUnion) (json.RawMessage, bool) {
	if item.AsAny() != nil {
		if data, err := json.Marshal(item.AsAny()); err == nil {
			return data, true
		}
	}
	if data, err := json.Marshal(item); err == nil {
		return data, true
	}
	return nil, false
}

func providerItemsFromRaw(rawItems []json.RawMessage) []Item {
	items := make([]Item, 0, len(rawItems))
	for _, raw := range rawItems {
		item := providerItemFromRaw(raw)
		if item.Kind == "" {
			item.Kind = ItemRaw
		}
		items = append(items, item)
	}
	return items
}

func providerItemFromRaw(raw json.RawMessage) Item {
	var object map[string]json.RawMessage
	if err := json.Unmarshal(raw, &object); err != nil {
		return Item{Kind: ItemRaw, RawJSON: append(json.RawMessage(nil), raw...)}
	}
	typ := rawString(object["type"])
	item := Item{
		ID:      rawString(object["id"]),
		Kind:    providerItemKind(typ),
		RawJSON: append(json.RawMessage(nil), raw...),
	}
	switch item.Kind {
	case ItemMessage:
		item.Role = rawString(object["role"])
		item.Text = messageText(object["content"])
	case ItemFunctionCall:
		item.ToolName = rawString(object["name"])
		item.ToolCallID = firstNonEmpty(rawString(object["call_id"]), item.ID)
		item.Text = rawString(object["arguments"])
	case ItemFunctionOut:
		item.ToolCallID = rawString(object["call_id"])
		item.Text = rawString(object["output"])
	case ItemReasoning:
		item.Text = firstNonEmpty(rawString(object["text"]), messageText(object["summary"]))
	}
	return item
}

func providerItemKind(typ string) ItemKind {
	switch typ {
	case "message":
		return ItemMessage
	case "function_call":
		return ItemFunctionCall
	case "function_call_output":
		return ItemFunctionOut
	case "reasoning":
		return ItemReasoning
	case "compaction":
		return ItemCompaction
	default:
		return ItemRaw
	}
}

func rawString(raw json.RawMessage) string {
	var value string
	_ = json.Unmarshal(raw, &value)
	return value
}

func messageText(raw json.RawMessage) string {
	var content []map[string]json.RawMessage
	if err := json.Unmarshal(raw, &content); err != nil {
		return rawString(raw)
	}
	parts := make([]string, 0, len(content))
	for _, part := range content {
		if text := firstNonEmpty(rawString(part["text"]), rawString(part["output_text"])); text != "" {
			parts = append(parts, text)
		}
	}
	return strings.Join(parts, "")
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
