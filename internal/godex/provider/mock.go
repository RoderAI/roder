package provider

import (
	"context"
	"encoding/json"
)

type Mock struct {
	finalText string
	tools     []ToolRequest
}

func NewMock(finalText string, tools []ToolRequest) *Mock {
	return &Mock{finalText: finalText, tools: tools}
}

func (m *Mock) Name() string {
	return "mock"
}

func (m *Mock) Stream(ctx context.Context, _ Request) (<-chan Event, <-chan error) {
	events := make(chan Event)
	errs := make(chan error, 1)
	go func() {
		defer close(events)
		defer close(errs)
		for _, tool := range m.tools {
			req := tool
			select {
			case <-ctx.Done():
				errs <- ctx.Err()
				return
			case events <- Event{Kind: EventToolCall, ToolRequest: &req}:
			}
		}
		if m.finalText != "" {
			select {
			case <-ctx.Done():
				errs <- ctx.Err()
				return
			case events <- Event{Kind: EventDelta, Text: m.finalText}:
			}
		}
		select {
		case <-ctx.Done():
			errs <- ctx.Err()
			return
		case events <- Event{Kind: EventCompleted, Text: m.finalText}:
		}
	}()
	return events, errs
}

func (m *Mock) Compact(ctx context.Context, _ CompactRequest) (CompactResult, error) {
	select {
	case <-ctx.Done():
		return CompactResult{}, ctx.Err()
	default:
	}
	return CompactResult{
		ID: "mock_compaction",
		Output: []json.RawMessage{
			json.RawMessage(`{"type":"compaction","encrypted_content":"mock","id":"cmp_mock"}`),
		},
	}, nil
}
