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
		toolCallIDs := map[string]string{}
		emittedToolItems := map[string]bool{}
		for stream.Next() {
			ev := stream.Current()
			switch ev.Type {
			case "response.output_text.delta":
				final += ev.Delta
				events <- Event{Kind: EventDelta, Text: ev.Delta}
			case "response.reasoning_summary_text.delta":
				if ev.Delta != "" {
					events <- Event{Kind: EventReasoningSummaryDelta, Text: ev.Delta}
				}
			case "response.reasoning_summary_text.done":
				if ev.Text != "" {
					events <- Event{Kind: EventReasoningSummaryDone, Text: ev.Text}
				}
			case "response.output_item.added":
				if ev.Item.Type == "function_call" {
					call := ev.Item.AsFunctionCall()
					toolNames[ev.Item.ID] = call.Name
					toolCallIDs[ev.Item.ID] = firstNonEmpty(call.CallID, call.ID, ev.Item.ID)
					toolArgs[ev.Item.ID] = call.Arguments
				}
			case "response.function_call_arguments.delta":
				toolArgs[ev.ItemID] += ev.Delta
			case "response.function_call_arguments.done":
				toolArgs[ev.ItemID] = firstNonEmpty(ev.Arguments, toolArgs[ev.ItemID])
				if name := firstNonEmpty(ev.Name, toolNames[ev.ItemID]); name != "" && !emittedToolItems[ev.ItemID] {
					arguments := toolArgs[ev.ItemID]
					events <- Event{Kind: EventToolCall, ToolRequest: &ToolRequest{
						ID:        firstNonEmpty(toolCallIDs[ev.ItemID], ev.ItemID),
						Name:      name,
						Input:     decodeArgs(arguments),
						Arguments: arguments,
					}}
					emittedToolItems[ev.ItemID] = true
				}
			case "response.output_item.done":
				if ev.Item.Type == "function_call" && !emittedToolItems[ev.Item.ID] {
					call := ev.Item.AsFunctionCall()
					callID := firstNonEmpty(call.CallID, toolCallIDs[ev.Item.ID], call.ID)
					events <- Event{Kind: EventToolCall, ToolRequest: &ToolRequest{
						ID:        callID,
						Name:      call.Name,
						Input:     decodeArgs(call.Arguments),
						Arguments: call.Arguments,
					}}
					emittedToolItems[ev.Item.ID] = true
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
			OfInputItemList: responseInputItems(req.Messages),
		},
		Reasoning: shared.ReasoningParam{
			Effort:  shared.ReasoningEffort(o.reasoning),
			Summary: shared.ReasoningSummaryAuto,
		},
		Store: param.NewOpt(false),
		Tools: openAITools(req.Tools),
	}
	if req.Instructions != "" {
		params.Instructions = param.NewOpt(req.Instructions)
	}
	if strings.TrimSpace(req.ResponseFormat) != "" {
		params.Text = responseTextConfig(req.ResponseFormat)
	}
	if o.serviceTier != "" {
		params.ServiceTier = responses.ResponseNewParamsServiceTier(o.serviceTier)
	}
	return params
}

func responseTextConfig(raw string) responses.ResponseTextConfigParam {
	trimmed := bytes.TrimSpace([]byte(raw))
	var object map[string]json.RawMessage
	if json.Unmarshal(trimmed, &object) == nil {
		if _, ok := object["format"]; ok {
			return param.Override[responses.ResponseTextConfigParam](json.RawMessage(trimmed))
		}
	}
	wrapped, _ := json.Marshal(map[string]json.RawMessage{"format": trimmed})
	return param.Override[responses.ResponseTextConfigParam](json.RawMessage(wrapped))
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

func responseInputItems(messages []Message) responses.ResponseInputParam {
	items := make(responses.ResponseInputParam, 0, len(messages))
	for _, msg := range messages {
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
		schema := normalizeToolSchema(spec.Schema)
		out = append(out, responses.ToolParamOfFunction(spec.Name, schema, false))
		if out[len(out)-1].OfFunction != nil {
			out[len(out)-1].OfFunction.Description = param.NewOpt(spec.Description)
		}
	}
	return out
}

func normalizeToolSchema(schema map[string]any) map[string]any {
	if schema == nil {
		return map[string]any{"type": "object", "properties": map[string]any{}}
	}
	normalized, ok := normalizeSchemaValue(schema).(map[string]any)
	if !ok {
		return map[string]any{"type": "object", "properties": map[string]any{}}
	}
	return normalized
}

func normalizeSchemaValue(value any) any {
	switch typed := value.(type) {
	case map[string]any:
		out := make(map[string]any, len(typed))
		for key, item := range typed {
			out[key] = normalizeSchemaValue(item)
		}
		if required, ok := out["required"]; ok && required == nil {
			out["required"] = []any{}
		}
		return out
	case []any:
		if typed == nil {
			return []any{}
		}
		out := make([]any, len(typed))
		for i, item := range typed {
			out[i] = normalizeSchemaValue(item)
		}
		return out
	case []string:
		if typed == nil {
			return []string{}
		}
		return append([]string{}, typed...)
	default:
		return typed
	}
}
