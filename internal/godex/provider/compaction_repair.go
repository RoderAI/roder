package provider

import (
	"encoding/json"
	"regexp"
)

var orphanFunctionCallOutputRE = regexp.MustCompile(`No tool call found for function call output with call_id ([^".,\s]+)`)

func RepairOrphanFunctionCallOutput(messages []Message, err error) ([]Message, string, bool) {
	if err == nil {
		return nil, "", false
	}
	callID := OrphanFunctionCallOutputID(err.Error())
	if callID == "" {
		return nil, "", false
	}
	repaired := make([]Message, 0, len(messages))
	removed := 0
	for _, msg := range messages {
		if isFunctionCallOutput(msg, callID) {
			removed++
			continue
		}
		repaired = append(repaired, msg)
	}
	if removed == 0 {
		return nil, "", false
	}
	return repaired, callID, true
}

func RepairOrphanFunctionCallOutputItems(items []Item, err error) ([]Item, string, bool) {
	if err == nil {
		return nil, "", false
	}
	callID := OrphanFunctionCallOutputID(err.Error())
	if callID == "" {
		return nil, "", false
	}
	repaired := make([]Item, 0, len(items))
	removed := 0
	for _, item := range items {
		if functionCallOutputItemID(item) == callID {
			removed++
			continue
		}
		repaired = append(repaired, item)
	}
	if removed == 0 {
		return nil, "", false
	}
	return repaired, callID, true
}

func RepairAllOrphanFunctionCallOutputs(messages []Message) ([]Message, []string, bool) {
	seenCalls := map[string]bool{}
	repaired := make([]Message, 0, len(messages))
	var removed []string
	for _, msg := range messages {
		if callID := functionCallID(msg); callID != "" {
			seenCalls[callID] = true
			repaired = append(repaired, msg)
			continue
		}
		if callID := functionCallOutputID(msg); callID != "" {
			if seenCalls[callID] {
				repaired = append(repaired, msg)
				continue
			}
			removed = append(removed, callID)
			continue
		}
		repaired = append(repaired, msg)
	}
	if len(removed) == 0 {
		return nil, nil, false
	}
	return repaired, removed, true
}

func RepairAllOrphanFunctionCallOutputItems(items []Item) ([]Item, []string, bool) {
	seenCalls := map[string]bool{}
	repaired := make([]Item, 0, len(items))
	var removed []string
	for _, item := range items {
		if callID := functionCallItemID(item); callID != "" {
			seenCalls[callID] = true
			repaired = append(repaired, item)
			continue
		}
		if callID := functionCallOutputItemID(item); callID != "" {
			if seenCalls[callID] {
				repaired = append(repaired, item)
				continue
			}
			removed = append(removed, callID)
			continue
		}
		repaired = append(repaired, item)
	}
	if len(removed) == 0 {
		return nil, nil, false
	}
	return repaired, removed, true
}

func OrphanFunctionCallOutputID(message string) string {
	match := orphanFunctionCallOutputRE.FindStringSubmatch(message)
	if len(match) != 2 {
		return ""
	}
	return match[1]
}

func isFunctionCallOutput(msg Message, callID string) bool {
	return functionCallOutputID(msg) == callID
}

func functionCallID(msg Message) string {
	if msg.Role == RoleAssistant && msg.ToolCallID != "" && msg.ToolName != "" {
		return msg.ToolCallID
	}
	if len(msg.RawJSON) == 0 {
		return ""
	}
	var object map[string]json.RawMessage
	if err := json.Unmarshal(msg.RawJSON, &object); err != nil {
		return ""
	}
	if rawString(object["type"]) != "function_call" {
		return ""
	}
	if callID := rawString(object["call_id"]); callID != "" {
		return callID
	}
	return rawString(object["id"])
}

func functionCallOutputID(msg Message) string {
	if msg.Role == RoleTool && msg.ToolCallID != "" {
		return msg.ToolCallID
	}
	if len(msg.RawJSON) == 0 {
		return ""
	}
	var object map[string]json.RawMessage
	if err := json.Unmarshal(msg.RawJSON, &object); err != nil {
		return ""
	}
	if rawString(object["type"]) != "function_call_output" {
		return ""
	}
	return rawString(object["call_id"])
}

func functionCallItemID(item Item) string {
	if item.Kind == ItemFunctionCall && item.ToolCallID != "" {
		return item.ToolCallID
	}
	if len(item.RawJSON) == 0 {
		return ""
	}
	var object map[string]json.RawMessage
	if err := json.Unmarshal(item.RawJSON, &object); err != nil {
		return ""
	}
	if rawString(object["type"]) != "function_call" {
		return ""
	}
	if callID := rawString(object["call_id"]); callID != "" {
		return callID
	}
	return rawString(object["id"])
}

func functionCallOutputItemID(item Item) string {
	if item.Kind == ItemFunctionOut && item.ToolCallID != "" {
		return item.ToolCallID
	}
	if len(item.RawJSON) == 0 {
		return ""
	}
	var object map[string]json.RawMessage
	if err := json.Unmarshal(item.RawJSON, &object); err != nil {
		return ""
	}
	if rawString(object["type"]) != "function_call_output" {
		return ""
	}
	return rawString(object["call_id"])
}
