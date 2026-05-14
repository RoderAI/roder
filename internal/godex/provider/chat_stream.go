package provider

import (
	"encoding/json"
	"fmt"
	"sort"
	"strings"
)

type chatStreamState struct {
	text         strings.Builder
	pendingTools map[int]*chatPendingTool
	emittedTools map[int]bool
	usage        TokenUsage
	done         bool
}

type chatPendingTool struct {
	id        string
	name      string
	arguments strings.Builder
}

type chatCompletionChunk struct {
	Choices []chatCompletionChoice `json:"choices"`
	Usage   chatUsage              `json:"usage"`
}

type chatCompletionChoice struct {
	Delta        chatDelta   `json:"delta"`
	Message      chatMessage `json:"message"`
	FinishReason string      `json:"finish_reason"`
}

type chatDelta struct {
	Content   string          `json:"content"`
	ToolCalls []chatToolDelta `json:"tool_calls"`
}

type chatMessage struct {
	Role      string         `json:"role"`
	Content   string         `json:"content"`
	ToolCalls []ChatToolCall `json:"tool_calls"`
}

type chatToolDelta struct {
	Index    int                  `json:"index"`
	ID       string               `json:"id"`
	Type     string               `json:"type"`
	Function chatToolFunctionPart `json:"function"`
}

type chatToolFunctionPart struct {
	Name      string `json:"name"`
	Arguments string `json:"arguments"`
}

type chatUsage struct {
	PromptTokens     int64 `json:"prompt_tokens"`
	CompletionTokens int64 `json:"completion_tokens"`
	TotalTokens      int64 `json:"total_tokens"`
}

func newChatStreamState() *chatStreamState {
	return &chatStreamState{
		pendingTools: map[int]*chatPendingTool{},
		emittedTools: map[int]bool{},
	}
}

func (s *chatStreamState) HandleChatSSELine(line []byte) ([]Event, error) {
	payload := strings.TrimSpace(string(line))
	if payload == "" || strings.HasPrefix(payload, ":") {
		return nil, nil
	}
	if strings.HasPrefix(payload, "data:") {
		payload = strings.TrimSpace(strings.TrimPrefix(payload, "data:"))
	}
	if payload == "[DONE]" {
		s.done = true
		return []Event{s.CompletedEvent()}, nil
	}
	var chunk chatCompletionChunk
	if err := json.Unmarshal([]byte(payload), &chunk); err != nil {
		return nil, fmt.Errorf("malformed chat completion stream chunk %q: %w", redactedChunkPrefix(payload), err)
	}
	return s.handleChunk(chunk), nil
}

func (s *chatStreamState) HandleChatCompletionResponse(data []byte) ([]Event, error) {
	events, err := s.HandleChatJSON(data)
	if err != nil {
		return nil, err
	}
	events = append(events, s.CompletedEvent())
	s.done = true
	return events, nil
}

func (s *chatStreamState) HandleChatJSON(data []byte) ([]Event, error) {
	var chunk chatCompletionChunk
	if err := json.Unmarshal(data, &chunk); err != nil {
		return nil, fmt.Errorf("malformed chat completion response %q: %w", redactedChunkPrefix(string(data)), err)
	}
	var out []Event
	for _, choice := range chunk.Choices {
		if choice.Message.Content != "" {
			s.text.WriteString(choice.Message.Content)
			out = append(out, Event{Kind: EventDelta, Text: choice.Message.Content})
		}
		for i, call := range choice.Message.ToolCalls {
			index := i
			pending := s.tool(index)
			pending.id = firstChatValue(call.ID, pending.id)
			pending.name = firstChatValue(call.Function.Name, pending.name)
			pending.arguments.WriteString(call.Function.Arguments)
		}
	}
	s.mergeUsage(chunk.Usage)
	out = append(out, s.emitPendingTools()...)
	return out, nil
}

func (s *chatStreamState) Done() bool {
	return s.done
}

func (s *chatStreamState) handleChunk(chunk chatCompletionChunk) []Event {
	var out []Event
	s.mergeUsage(chunk.Usage)
	for _, choice := range chunk.Choices {
		if choice.Delta.Content != "" {
			s.text.WriteString(choice.Delta.Content)
			out = append(out, Event{Kind: EventDelta, Text: choice.Delta.Content})
		}
		for _, call := range choice.Delta.ToolCalls {
			pending := s.tool(call.Index)
			pending.id = firstChatValue(call.ID, pending.id)
			pending.name = firstChatValue(call.Function.Name, pending.name)
			pending.arguments.WriteString(call.Function.Arguments)
		}
		if choice.FinishReason == "tool_calls" {
			out = append(out, s.emitPendingTools()...)
		}
	}
	return out
}

func (s *chatStreamState) tool(index int) *chatPendingTool {
	pending := s.pendingTools[index]
	if pending == nil {
		pending = &chatPendingTool{}
		s.pendingTools[index] = pending
	}
	return pending
}

func (s *chatStreamState) emitPendingTools() []Event {
	indexes := make([]int, 0, len(s.pendingTools))
	for index := range s.pendingTools {
		if !s.emittedTools[index] {
			indexes = append(indexes, index)
		}
	}
	sort.Ints(indexes)
	out := make([]Event, 0, len(indexes))
	for _, index := range indexes {
		pending := s.pendingTools[index]
		if pending == nil {
			continue
		}
		s.emittedTools[index] = true
		args := pending.arguments.String()
		item := Item{
			ID:         pending.id,
			Kind:       ItemFunctionCall,
			ToolName:   pending.name,
			ToolCallID: pending.id,
			Text:       args,
		}
		out = append(out, Event{
			Kind: EventToolCall,
			ToolRequest: &ToolRequest{
				ID:        pending.id,
				Name:      pending.name,
				Input:     decodeArgs(args),
				Arguments: args,
			},
			Items: []Item{item},
		})
	}
	return out
}

func (s *chatStreamState) CompletedEvent() Event {
	return Event{
		Kind:  EventCompleted,
		Text:  s.text.String(),
		Items: s.completedItems(),
		Usage: s.usage,
	}
}

func (s *chatStreamState) completedItems() []Item {
	items := []Item{}
	if text := s.text.String(); strings.TrimSpace(text) != "" {
		items = append(items, Item{Kind: ItemMessage, Role: string(RoleAssistant), Text: text})
	}
	indexes := make([]int, 0, len(s.pendingTools))
	for index := range s.pendingTools {
		indexes = append(indexes, index)
	}
	sort.Ints(indexes)
	for _, index := range indexes {
		pending := s.pendingTools[index]
		if pending == nil {
			continue
		}
		items = append(items, Item{
			ID:         pending.id,
			Kind:       ItemFunctionCall,
			ToolName:   pending.name,
			ToolCallID: pending.id,
			Text:       pending.arguments.String(),
		})
	}
	return items
}

func (s *chatStreamState) mergeUsage(usage chatUsage) {
	if usage.PromptTokens > 0 {
		s.usage.InputTokens = usage.PromptTokens
	}
	if usage.CompletionTokens > 0 {
		s.usage.OutputTokens = usage.CompletionTokens
	}
	if usage.TotalTokens > 0 {
		s.usage.TotalTokens = usage.TotalTokens
	} else if s.usage.InputTokens > 0 || s.usage.OutputTokens > 0 {
		s.usage.TotalTokens = s.usage.InputTokens + s.usage.OutputTokens
	}
}

func redactedChunkPrefix(payload string) string {
	payload = strings.TrimSpace(payload)
	const max = 64
	if len(payload) <= max {
		return payload
	}
	return payload[:max] + "...<truncated>"
}
