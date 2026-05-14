package provider

import (
	"encoding/json"

	"github.com/pandelisz/gode/internal/godex/contextwindow"
)

const LocalPruneMarkerText = "Earlier context was pruned locally because remote compaction failed."

type LocalPruneRequest struct {
	Model          string
	Instructions   string
	ResponseFormat string
	Messages       []Message
	Tools          []ToolSpec
	TargetTokens   int
}

func LocalPrunedMessages(req LocalPruneRequest) ([]Message, int, bool) {
	if len(req.Messages) == 0 {
		return nil, 0, false
	}
	window := contextwindow.ForModel(req.Model)
	target := req.TargetTokens
	if target <= 0 {
		target = window.AutoCompactTokenLimit
	}
	if target <= 0 && window.ContextWindow > 0 {
		target = window.ContextWindow / 2
	}
	marker := LocalPruneMarkerMessage()
	kept := make([]Message, 0, len(req.Messages))
	dropped := 0
	for i := len(req.Messages) - 1; i >= 0; i-- {
		candidate := make([]Message, 0, len(kept)+2)
		candidate = append(candidate, marker, req.Messages[i])
		candidate = append(candidate, kept...)
		if len(kept) == 0 || target <= 0 || localPruneEstimate(req, candidate, window) <= target {
			kept = append([]Message{req.Messages[i]}, kept...)
			continue
		}
		dropped++
	}
	if dropped == 0 {
		return nil, 0, false
	}
	pruned := make([]Message, 0, len(kept)+1)
	pruned = append(pruned, marker)
	pruned = append(pruned, kept...)
	return pruned, dropped, true
}

func LocalPruneMarkerMessage() Message {
	raw, _ := json.Marshal(map[string]any{
		"type": "message",
		"role": "user",
		"content": []map[string]string{{
			"type": "input_text",
			"text": LocalPruneMarkerText,
		}},
	})
	return Message{RawJSON: raw}
}

func localPruneEstimate(req LocalPruneRequest, messages []Message, window contextwindow.ModelWindow) int {
	estimate := contextwindow.EstimateRequest(contextwindow.Request{
		Model:          req.Model,
		Instructions:   req.Instructions,
		ResponseFormat: req.ResponseFormat,
		Messages:       contextPruneMessages(messages),
		Tools:          contextPruneTools(req.Tools),
	}, window)
	return estimate.Tokens
}

func contextPruneMessages(messages []Message) []contextwindow.Message {
	out := make([]contextwindow.Message, 0, len(messages))
	for _, msg := range messages {
		out = append(out, contextwindow.Message{
			Role:          string(msg.Role),
			Content:       msg.Content,
			Phase:         msg.Phase,
			Images:        contextPruneImages(msg.Images),
			ToolCallID:    msg.ToolCallID,
			ToolName:      msg.ToolName,
			ToolArguments: msg.ToolArguments,
			RawJSON:       msg.RawJSON,
		})
	}
	return out
}

func contextPruneTools(tools []ToolSpec) []contextwindow.ToolSpec {
	out := make([]contextwindow.ToolSpec, 0, len(tools))
	for _, tool := range tools {
		out = append(out, contextwindow.ToolSpec{
			Name:        tool.Name,
			Description: tool.Description,
			Schema:      tool.Schema,
		})
	}
	return out
}

func contextPruneImages(images []Image) []contextwindow.Image {
	out := make([]contextwindow.Image, 0, len(images))
	for _, image := range images {
		out = append(out, contextwindow.Image{URL: image.URL, Detail: image.Detail})
	}
	return out
}
