package provider

import (
	"encoding/json"
	"fmt"
	"strings"

	"google.golang.org/genai"
)

type geminiStreamState struct {
	text         strings.Builder
	pendingTools map[string]*geminiPendingTool
	toolOrder    []string
	emittedTools map[string]bool
	usage        TokenUsage
	finishReason string
	responseID   string
}

type geminiPendingTool struct {
	id        string
	name      string
	arguments string
	input     map[string]any
	raw       json.RawMessage
	signature []byte
}

func newGeminiStreamState() *geminiStreamState {
	return &geminiStreamState{pendingTools: map[string]*geminiPendingTool{}, emittedTools: map[string]bool{}}
}

func (s *geminiStreamState) Handle(resp *genai.GenerateContentResponse) ([]Event, error) {
	if resp == nil {
		return nil, nil
	}
	if resp.ResponseID != "" {
		s.responseID = resp.ResponseID
	}
	s.mergeUsage(resp.UsageMetadata)
	if resp.PromptFeedback != nil && resp.PromptFeedback.BlockReason != "" {
		return nil, fmt.Errorf("Gemini prompt blocked: %s %s", resp.PromptFeedback.BlockReason, strings.TrimSpace(resp.PromptFeedback.BlockReasonMessage))
	}
	var out []Event
	for _, candidate := range resp.Candidates {
		if candidate == nil {
			continue
		}
		if candidate.FinishReason != "" && candidate.FinishReason != genai.FinishReasonStop {
			s.finishReason = string(candidate.FinishReason)
		}
		if candidate.FinishReason == genai.FinishReasonSafety || candidate.FinishReason == genai.FinishReasonMalformedFunctionCall || candidate.FinishReason == genai.FinishReasonUnexpectedToolCall {
			return nil, fmt.Errorf("Gemini candidate stopped: %s %s", candidate.FinishReason, strings.TrimSpace(candidate.FinishMessage))
		}
		if candidate.Content == nil {
			continue
		}
		for _, part := range candidate.Content.Parts {
			if part == nil {
				continue
			}
			if part.Text != "" && !part.Thought {
				s.text.WriteString(part.Text)
				out = append(out, Event{Kind: EventDelta, Text: part.Text})
			}
			if part.FunctionCall != nil {
				out = append(out, s.handleFunctionCall(part)...)
			}
		}
	}
	return out, nil
}

func (s *geminiStreamState) handleFunctionCall(part *genai.Part) []Event {
	call := part.FunctionCall
	id := firstNonEmpty(call.ID, call.Name)
	if id == "" {
		id = fmt.Sprintf("gemini_call_%d", len(s.toolOrder)+1)
	}
	pending := s.pendingTools[id]
	if pending == nil {
		pending = &geminiPendingTool{id: id}
		s.pendingTools[id] = pending
		s.toolOrder = append(s.toolOrder, id)
	}
	pending.name = firstNonEmpty(call.Name, pending.name)
	pending.input = call.Args
	if len(part.ThoughtSignature) > 0 {
		pending.signature = append([]byte(nil), part.ThoughtSignature...)
	}
	data, _ := json.Marshal(call.Args)
	pending.arguments = string(data)
	if raw, err := json.Marshal(part); err == nil {
		pending.raw = raw
	}
	if s.emittedTools[id] {
		return nil
	}
	s.emittedTools[id] = true
	return []Event{{
		Kind: EventToolCall,
		ToolRequest: &ToolRequest{
			ID:        pending.id,
			Name:      pending.name,
			Input:     pending.input,
			Arguments: pending.arguments,
		},
		Items: []Item{{
			ID:         pending.id,
			Kind:       ItemFunctionCall,
			ToolName:   pending.name,
			ToolCallID: pending.id,
			Text:       pending.arguments,
			RawJSON:    pending.raw,
		}},
	}}
}

func (s *geminiStreamState) CompletedEvent() Event {
	return Event{Kind: EventCompleted, Text: s.text.String(), ResponseID: s.responseID, Items: s.completedItems(), Usage: s.usage}
}

func (s *geminiStreamState) completedItems() []Item {
	items := []Item{}
	if text := s.text.String(); strings.TrimSpace(text) != "" {
		items = append(items, Item{Kind: ItemMessage, Role: string(RoleAssistant), Text: text})
	}
	for _, id := range s.toolOrder {
		pending := s.pendingTools[id]
		if pending == nil {
			continue
		}
		items = append(items, Item{ID: pending.id, Kind: ItemFunctionCall, ToolName: pending.name, ToolCallID: pending.id, Text: pending.arguments, RawJSON: pending.raw})
	}
	return items
}

func (s *geminiStreamState) mergeUsage(usage *genai.GenerateContentResponseUsageMetadata) {
	if usage == nil {
		return
	}
	if usage.PromptTokenCount > 0 {
		s.usage.InputTokens = int64(usage.PromptTokenCount)
	}
	if usage.CandidatesTokenCount > 0 {
		s.usage.OutputTokens = int64(usage.CandidatesTokenCount)
	}
	if usage.TotalTokenCount > 0 {
		s.usage.TotalTokens = int64(usage.TotalTokenCount)
	} else if s.usage.InputTokens > 0 || s.usage.OutputTokens > 0 {
		s.usage.TotalTokens = s.usage.InputTokens + s.usage.OutputTokens
	}
}
