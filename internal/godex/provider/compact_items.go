package provider

import (
	"encoding/json"
	"strings"
	"unicode/utf8"

	"github.com/openai/openai-go/v3/packages/param"
	"github.com/openai/openai-go/v3/responses"
)

const compactFunctionOutputLimitBytes = 1 << 20

const compactFunctionOutputMarker = "\n\n[... gode truncated this tool output for compaction; the full output remains in the event journal ...]\n\n"

func compactResponseInputItems(messages []Message) responses.ResponseInputParam {
	items := make(responses.ResponseInputParam, 0, len(messages))
	for _, msg := range messages {
		if len(msg.RawJSON) > 0 {
			items = append(items, param.Override[responses.ResponseInputItemUnionParam](compactRawResponseItem(msg.RawJSON)))
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
			output := compactFunctionOutput(strings.TrimSpace(msg.Content))
			items = append(items, responses.ResponseInputItemParamOfFunctionCallOutput(msg.ToolCallID, output))
			continue
		}
		content := strings.TrimSpace(msg.Content)
		if content == "" && len(msg.Images) == 0 {
			continue
		}
		items = append(items, responseInputMessageParam(content, msg.Role, msg.Phase, msg.Images))
	}
	return items
}

func compactRawResponseItem(raw json.RawMessage) json.RawMessage {
	var object map[string]json.RawMessage
	if err := json.Unmarshal(raw, &object); err != nil {
		return append(json.RawMessage(nil), raw...)
	}
	if rawString(object["type"]) != "function_call_output" {
		return append(json.RawMessage(nil), raw...)
	}
	output := rawString(object["output"])
	compacted := compactFunctionOutput(output)
	if compacted == output {
		return append(json.RawMessage(nil), raw...)
	}
	encoded, err := json.Marshal(compacted)
	if err != nil {
		return append(json.RawMessage(nil), raw...)
	}
	object["output"] = encoded
	out, err := json.Marshal(object)
	if err != nil {
		return append(json.RawMessage(nil), raw...)
	}
	return out
}

func compactFunctionOutput(output string) string {
	return truncateMiddleUTF8(output, compactFunctionOutputLimitBytes, compactFunctionOutputMarker)
}

func truncateMiddleUTF8(value string, limit int, marker string) string {
	if limit <= 0 || len(value) <= limit {
		return value
	}
	if len(marker) >= limit {
		return utf8Prefix(value, limit)
	}
	remaining := limit - len(marker)
	headBudget := remaining / 2
	tailBudget := remaining - headBudget
	return utf8Prefix(value, headBudget) + marker + utf8Suffix(value, tailBudget)
}

func utf8Prefix(value string, limit int) string {
	if limit <= 0 {
		return ""
	}
	if len(value) <= limit {
		return value
	}
	end := limit
	for end > 0 && !utf8.ValidString(value[:end]) {
		end--
	}
	return value[:end]
}

func utf8Suffix(value string, limit int) string {
	if limit <= 0 {
		return ""
	}
	if len(value) <= limit {
		return value
	}
	start := len(value) - limit
	for start < len(value) && !utf8.RuneStart(value[start]) {
		start++
	}
	return value[start:]
}
