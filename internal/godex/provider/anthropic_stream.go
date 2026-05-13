package provider

import (
	"encoding/json"
	"fmt"
	"strings"

	anthropic "github.com/anthropics/anthropic-sdk-go"
)

type anthropicStreamState struct {
	message           anthropic.Message
	text              strings.Builder
	pendingTools      map[int64]*anthropicPendingTool
	emittedTools      map[int64]bool
	malformedToolJSON bool
}

type anthropicPendingTool struct {
	id        string
	name      string
	arguments strings.Builder
	rawStart  json.RawMessage
}

func newAnthropicStreamState() *anthropicStreamState {
	return &anthropicStreamState{
		pendingTools: map[int64]*anthropicPendingTool{},
		emittedTools: map[int64]bool{},
	}
}

func (s *anthropicStreamState) Handle(ev anthropic.MessageStreamEventUnion) ([]Event, error) {
	if err := s.message.Accumulate(ev); err != nil {
		if !s.canContinueAfterAccumulateError(ev) {
			return nil, err
		}
		s.malformedToolJSON = true
	}
	switch ev.Type {
	case "content_block_start":
		return s.handleContentBlockStart(ev.AsContentBlockStart())
	case "content_block_delta":
		return s.handleContentBlockDelta(ev.AsContentBlockDelta())
	case "content_block_stop":
		return s.handleContentBlockStop(ev.AsContentBlockStop())
	default:
		return nil, nil
	}
}

func (s *anthropicStreamState) canContinueAfterAccumulateError(ev anthropic.MessageStreamEventUnion) bool {
	return (ev.Type == "content_block_stop" && s.pendingTools[ev.Index] != nil) ||
		(ev.Type == "message_stop" && s.malformedToolJSON)
}

func (s *anthropicStreamState) handleContentBlockStart(ev anthropic.ContentBlockStartEvent) ([]Event, error) {
	block := ev.ContentBlock
	switch block.Type {
	case "text":
		if block.Text == "" {
			return nil, nil
		}
		s.text.WriteString(block.Text)
		return []Event{{Kind: EventDelta, Text: block.Text}}, nil
	case "tool_use":
		pending := &anthropicPendingTool{
			id:       block.ID,
			name:     block.Name,
			rawStart: rawJSON(block.RawJSON()),
		}
		if raw := rawToolInput(block.Input); raw != "" && raw != "{}" {
			pending.arguments.WriteString(raw)
		}
		s.pendingTools[ev.Index] = pending
	}
	return nil, nil
}

func (s *anthropicStreamState) handleContentBlockDelta(ev anthropic.ContentBlockDeltaEvent) ([]Event, error) {
	switch ev.Delta.Type {
	case "text_delta":
		if ev.Delta.Text == "" {
			return nil, nil
		}
		s.text.WriteString(ev.Delta.Text)
		return []Event{{Kind: EventDelta, Text: ev.Delta.Text}}, nil
	case "input_json_delta":
		if pending := s.pendingTools[ev.Index]; pending != nil {
			pending.arguments.WriteString(ev.Delta.PartialJSON)
		}
	}
	return nil, nil
}

func (s *anthropicStreamState) handleContentBlockStop(ev anthropic.ContentBlockStopEvent) ([]Event, error) {
	if s.emittedTools[ev.Index] {
		return nil, nil
	}
	pending := s.pendingTools[ev.Index]
	if pending == nil {
		return nil, nil
	}
	s.emittedTools[ev.Index] = true
	args := pending.arguments.String()
	return []Event{{
		Kind: EventToolCall,
		ToolRequest: &ToolRequest{
			ID:        pending.id,
			Name:      pending.name,
			Input:     decodeArgs(args),
			Arguments: args,
		},
		Items: []Item{{
			ID:         pending.id,
			Kind:       ItemFunctionCall,
			ToolName:   pending.name,
			ToolCallID: pending.id,
			Text:       args,
			RawJSON:    pending.rawStart,
		}},
	}}, nil
}

func (s *anthropicStreamState) CompletedEvent() Event {
	return Event{
		Kind:  EventCompleted,
		Text:  s.text.String(),
		Items: anthropicItemsFromMessage(s.message),
	}
}

func anthropicItemsFromMessage(message anthropic.Message) []Item {
	items := make([]Item, 0, len(message.Content))
	for i, block := range message.Content {
		id := fmt.Sprintf("anthropic_%d", i)
		switch block.Type {
		case "text":
			if strings.TrimSpace(block.Text) == "" {
				continue
			}
			items = append(items, Item{
				ID:      id,
				Kind:    ItemMessage,
				Role:    string(RoleAssistant),
				Text:    block.Text,
				RawJSON: rawJSON(block.RawJSON()),
			})
		case "tool_use":
			args := string(block.Input)
			items = append(items, Item{
				ID:         firstNonEmpty(block.ID, id),
				Kind:       ItemFunctionCall,
				ToolName:   block.Name,
				ToolCallID: firstNonEmpty(block.ID, id),
				Text:       args,
				RawJSON:    rawJSON(block.RawJSON()),
			})
		}
	}
	return items
}

func rawToolInput(input any) string {
	data, err := json.Marshal(input)
	if err != nil {
		return ""
	}
	return string(data)
}

func rawJSON(text string) json.RawMessage {
	if strings.TrimSpace(text) == "" {
		return nil
	}
	return json.RawMessage(text)
}
