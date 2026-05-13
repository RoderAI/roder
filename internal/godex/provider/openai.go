package provider

import (
	"context"
	"encoding/json"

	openai "github.com/openai/openai-go/v3"
	"github.com/openai/openai-go/v3/option"
	"github.com/openai/openai-go/v3/packages/param"
	"github.com/openai/openai-go/v3/responses"
	"github.com/openai/openai-go/v3/shared"
)

type OpenAI struct {
	client    openai.Client
	model     string
	reasoning string
}

func NewOpenAI(model string, reasoning string, opts ...option.RequestOption) *OpenAI {
	if model == "" {
		model = "gpt-5.4-mini"
	}
	if reasoning == "" {
		reasoning = "low"
	}
	return &OpenAI{client: openai.NewClient(opts...), model: model, reasoning: reasoning}
}

func (o *OpenAI) Name() string {
	return "openai"
}

func (o *OpenAI) Stream(ctx context.Context, req Request) (<-chan Event, <-chan error) {
	events := make(chan Event)
	errs := make(chan error, 1)
	go func() {
		defer close(events)
		defer close(errs)
		params := responses.ResponseNewParams{
			Model: responses.ResponsesModel(o.model),
			Input: responses.ResponseNewParamsInputUnion{
				OfString: param.NewOpt(inputString(req.Messages)),
			},
			Reasoning: shared.ReasoningParam{Effort: shared.ReasoningEffort(o.reasoning)},
			Tools:     openAITools(req.Tools),
		}
		stream := o.client.Responses.NewStreaming(ctx, params)
		defer stream.Close()
		final := ""
		toolArgs := map[string]string{}
		for stream.Next() {
			ev := stream.Current()
			switch ev.Type {
			case "response.output_text.delta":
				final += ev.Delta
				events <- Event{Kind: EventDelta, Text: ev.Delta}
			case "response.function_call_arguments.delta":
				toolArgs[ev.ItemID] += ev.Delta
			case "response.function_call_arguments.done":
				args := map[string]any{}
				raw := ev.Arguments
				if raw == "" {
					raw = toolArgs[ev.ItemID]
				}
				_ = json.Unmarshal([]byte(raw), &args)
				events <- Event{Kind: EventToolCall, ToolRequest: &ToolRequest{ID: ev.ItemID, Name: ev.Name, Input: args}}
			case "response.completed":
				if final == "" {
					final = ev.Response.OutputText()
				}
				events <- Event{Kind: EventCompleted, Text: final}
			case "response.failed", "error":
				errs <- &ProviderError{Message: ev.Message}
				return
			}
		}
		if err := stream.Err(); err != nil {
			errs <- err
			return
		}
	}()
	return events, errs
}

type ProviderError struct {
	Message string
}

func (e *ProviderError) Error() string {
	if e.Message == "" {
		return "provider error"
	}
	return e.Message
}

func inputString(messages []Message) string {
	var out string
	for _, msg := range messages {
		if msg.Content == "" {
			continue
		}
		if out != "" {
			out += "\n\n"
		}
		out += string(msg.Role) + ": " + msg.Content
	}
	return out
}

func openAITools(specs []ToolSpec) []responses.ToolUnionParam {
	out := make([]responses.ToolUnionParam, 0, len(specs))
	for _, spec := range specs {
		schema := spec.Schema
		if schema == nil {
			schema = map[string]any{"type": "object", "properties": map[string]any{}}
		}
		out = append(out, responses.ToolParamOfFunction(spec.Name, schema, false))
		if out[len(out)-1].OfFunction != nil {
			out[len(out)-1].OfFunction.Description = param.NewOpt(spec.Description)
		}
	}
	return out
}
