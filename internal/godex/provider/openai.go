package provider

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"strings"

	openai "github.com/openai/openai-go/v3"
	"github.com/openai/openai-go/v3/option"
	"github.com/openai/openai-go/v3/packages/param"
	"github.com/openai/openai-go/v3/responses"
	"github.com/openai/openai-go/v3/shared"
)

type OpenAI struct {
	client      openai.Client
	model       string
	reasoning   string
	serviceTier string
}

type OpenAIConfig struct {
	Model       string
	Reasoning   string
	ServiceTier string
}

func NewOpenAI(model string, reasoning string, opts ...option.RequestOption) *OpenAI {
	return NewOpenAIWithConfig(OpenAIConfig{Model: model, Reasoning: reasoning}, opts...)
}

func NewOpenAIWithConfig(cfg OpenAIConfig, opts ...option.RequestOption) *OpenAI {
	if cfg.Model == "" {
		cfg.Model = "gpt-5.5"
	}
	if cfg.Reasoning == "" {
		cfg.Reasoning = "medium"
	}
	return &OpenAI{
		client:      openai.NewClient(opts...),
		model:       cfg.Model,
		reasoning:   cfg.Reasoning,
		serviceTier: cfg.ServiceTier,
	}
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
		params := o.responseParams(req)
		stream := o.client.Responses.NewStreaming(ctx, params)
		defer stream.Close()
		final := ""
		toolArgs := map[string]string{}
		toolNames := map[string]string{}
		emittedTools := map[string]bool{}
		for stream.Next() {
			ev := stream.Current()
			switch ev.Type {
			case "response.output_text.delta":
				final += ev.Delta
				events <- Event{Kind: EventDelta, Text: ev.Delta}
			case "response.output_item.added":
				if ev.Item.Type == "function_call" {
					toolNames[ev.Item.ID] = ev.Item.Name
				}
			case "response.function_call_arguments.delta":
				toolArgs[ev.ItemID] += ev.Delta
			case "response.function_call_arguments.done":
				toolArgs[ev.ItemID] = firstNonEmpty(ev.Arguments, toolArgs[ev.ItemID])
				if name := firstNonEmpty(ev.Name, toolNames[ev.ItemID]); name != "" && !emittedTools[ev.ItemID] {
					events <- Event{Kind: EventToolCall, ToolRequest: &ToolRequest{ID: ev.ItemID, Name: name, Input: decodeArgs(toolArgs[ev.ItemID])}}
					emittedTools[ev.ItemID] = true
				}
			case "response.output_item.done":
				if ev.Item.Type == "function_call" && !emittedTools[ev.Item.ID] {
					call := ev.Item.AsFunctionCall()
					events <- Event{Kind: EventToolCall, ToolRequest: &ToolRequest{ID: call.ID, Name: call.Name, Input: decodeArgs(call.Arguments)}}
					emittedTools[call.ID] = true
				}
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
			errs <- o.detailedStreamError(err, req)
			return
		}
	}()
	return events, errs
}

func (o *OpenAI) responseParams(req Request) responses.ResponseNewParams {
	params := responses.ResponseNewParams{
		Model: responses.ResponsesModel(o.model),
		Input: responses.ResponseNewParamsInputUnion{
			OfString: param.NewOpt(inputString(req.Messages)),
		},
		Reasoning: shared.ReasoningParam{Effort: shared.ReasoningEffort(o.reasoning)},
		Tools:     openAITools(req.Tools),
	}
	if req.Instructions != "" {
		params.Instructions = param.NewOpt(req.Instructions)
	}
	if o.serviceTier != "" {
		params.ServiceTier = responses.ResponseNewParamsServiceTier(o.serviceTier)
	}
	return params
}

func (o *OpenAI) detailedStreamError(err error, req Request) error {
	detail := o.formatStreamError(err, req)
	if detail == "" {
		return err
	}
	return &ProviderError{Message: detail}
}

func (o *OpenAI) formatStreamError(err error, req Request) string {
	lines := []string{"OpenAI stream request failed"}
	var apiErr *openai.Error
	if errors.As(err, &apiErr) {
		if apiErr.Request != nil {
			lines = append(lines, "request: "+apiErr.Request.Method+" "+apiErr.Request.URL.String())
		}
		statusCode := apiErr.StatusCode
		if statusCode == 0 && apiErr.Response != nil {
			statusCode = apiErr.Response.StatusCode
		}
		if statusCode != 0 {
			lines = append(lines, fmt.Sprintf("status: %d %s", statusCode, http.StatusText(statusCode)))
		}
		if apiErr.Response != nil {
			for _, name := range []string{"x-request-id", "openai-request-id", "cf-ray"} {
				if value := apiErr.Response.Header.Get(name); value != "" {
					lines = append(lines, name+": "+value)
				}
			}
		}
		if apiErr.Type != "" {
			lines = append(lines, "error_type: "+apiErr.Type)
		}
		if apiErr.Code != "" {
			lines = append(lines, "error_code: "+apiErr.Code)
		}
		if apiErr.Param != "" {
			lines = append(lines, "error_param: "+apiErr.Param)
		}
		if apiErr.Message != "" {
			lines = append(lines, "error_message: "+apiErr.Message)
		}
		if raw := strings.TrimSpace(apiErr.RawJSON()); raw != "" {
			lines = append(lines, "raw_error_json: "+raw)
		}
		if body := readAPIErrorBody(apiErr); body != "" {
			lines = append(lines, "response_body:", body)
		}
	} else {
		lines = append(lines, "error: "+err.Error())
	}
	lines = append(lines,
		"model: "+o.model,
		"reasoning: "+o.reasoning,
		fmt.Sprintf("messages: %d", len(req.Messages)),
		fmt.Sprintf("input_chars: %d", inputChars(req.Messages)),
		fmt.Sprintf("tools: %d", len(req.Tools)),
	)
	if o.serviceTier != "" {
		lines = append(lines, "service_tier: "+o.serviceTier)
	}
	if len(req.Tools) > 0 {
		lines = append(lines, "tool_names: "+toolNames(req.Tools))
	}
	lines = append(lines, "sdk_error: "+err.Error())
	return strings.Join(lines, "\n")
}

func readAPIErrorBody(apiErr *openai.Error) string {
	if apiErr == nil || apiErr.Response == nil || apiErr.Response.Body == nil {
		return ""
	}
	const maxBody = 64 * 1024
	data, _ := io.ReadAll(io.LimitReader(apiErr.Response.Body, maxBody+1))
	apiErr.Response.Body = io.NopCloser(bytes.NewBuffer(data))
	text := strings.TrimSpace(string(data))
	if len(data) > maxBody {
		text = strings.TrimSpace(string(data[:maxBody])) + "\n... truncated"
	}
	return text
}

func inputChars(messages []Message) int {
	total := 0
	for _, msg := range messages {
		total += len(msg.Content)
	}
	return total
}

func toolNames(tools []ToolSpec) string {
	names := make([]string, 0, len(tools))
	for _, tool := range tools {
		if tool.Name != "" {
			names = append(names, tool.Name)
		}
	}
	return strings.Join(names, ", ")
}

func decodeArgs(raw string) map[string]any {
	args := map[string]any{}
	_ = json.Unmarshal([]byte(raw), &args)
	return args
}

func firstNonEmpty(values ...string) string {
	for _, value := range values {
		if value != "" {
			return value
		}
	}
	return ""
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
