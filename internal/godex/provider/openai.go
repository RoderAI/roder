package provider

import (
	"bytes"
	"context"
	"encoding/json"
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
	baseURL     string
	apiKey      string
	headers     map[string]string
}

type OpenAIConfig struct {
	Model       string
	Reasoning   string
	ServiceTier string
	BaseURL     string
	APIKey      string
	Headers     map[string]string
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
	if cfg.BaseURL != "" {
		opts = append([]option.RequestOption{option.WithBaseURL(cfg.BaseURL)}, opts...)
	}
	if cfg.APIKey != "" {
		opts = append([]option.RequestOption{option.WithAPIKey(cfg.APIKey)}, opts...)
	}
	for key, value := range cfg.Headers {
		if strings.TrimSpace(key) != "" {
			opts = append(opts, option.WithHeader(key, value))
		}
	}
	return &OpenAI{
		client:      openai.NewClient(opts...),
		model:       cfg.Model,
		reasoning:   cfg.Reasoning,
		serviceTier: cfg.ServiceTier,
		baseURL:     cfg.BaseURL,
		apiKey:      cfg.APIKey,
		headers:     cloneStringMap(cfg.Headers),
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
		messagePhases := map[string]string{}
		currentMessagePhase := ""
		for stream.Next() {
			ev := stream.Current()
			switch ev.Type {
			case "response.output_text.delta":
				phase := firstNonEmpty(messagePhases[ev.ItemID], currentMessagePhase)
				if phase == "" || phase == PhaseFinalAnswer {
					final += ev.Delta
				}
				events <- Event{Kind: EventDelta, Text: ev.Delta, Phase: phase}
			case "response.reasoning_summary_text.delta":
				if ev.Delta != "" {
					events <- Event{Kind: EventReasoningSummaryDelta, Text: ev.Delta}
				}
			case "response.reasoning_summary_text.done":
				if ev.Text != "" {
					events <- Event{Kind: EventReasoningSummaryDone, Text: ev.Text}
				}
			case "response.output_item.added":
				if ev.Item.Type == "message" {
					msg := ev.Item.AsMessage()
					currentMessagePhase = string(msg.Phase)
					messagePhases[ev.Item.ID] = currentMessagePhase
				}
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
					var items []Item
					if raw, ok := rawResponseOutputItem(ev.Item); ok {
						items = providerItemsFromRaw([]json.RawMessage{raw})
					}
					events <- Event{Kind: EventToolCall, ToolRequest: &ToolRequest{
						ID:        callID,
						Name:      call.Name,
						Input:     decodeArgs(call.Arguments),
						Arguments: call.Arguments,
					}, Items: items}
					emittedToolItems[ev.Item.ID] = true
				}
			case "response.completed":
				rawItems := rawResponseOutputItems(ev.Response.Output)
				if final == "" {
					final = finalAnswerTextFromRaw(rawItems)
				}
				if final == "" {
					final = ev.Response.OutputText()
				}
				events <- Event{
					Kind:       EventCompleted,
					Text:       final,
					ResponseID: ev.Response.ID,
					Items:      providerItemsFromRaw(rawItems),
					Usage:      openAITokenUsage(ev.Response.Usage),
				}
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

func openAITokenUsage(usage responses.ResponseUsage) TokenUsage {
	out := TokenUsage{
		InputTokens:  usage.InputTokens,
		OutputTokens: usage.OutputTokens,
		TotalTokens:  usage.TotalTokens,
	}
	if out.TotalTokens == 0 {
		out.TotalTokens = out.InputTokens + out.OutputTokens
	}
	return out
}

func (o *OpenAI) Compact(ctx context.Context, req CompactRequest) (CompactResult, error) {
	params := o.compactParams(req)
	result, err := o.client.Responses.Compact(ctx, params)
	if err != nil {
		return CompactResult{}, err
	}
	return CompactResult{ID: result.ID, Output: rawResponseOutputItems(result.Output)}, nil
}

func (o *OpenAI) compactParams(req CompactRequest) responses.ResponseCompactParams {
	params := responses.ResponseCompactParams{
		Model: responses.ResponseCompactParamsModel(firstNonEmpty(req.Model, o.model)),
		Input: responses.ResponseCompactParamsInputUnion{
			OfResponseInputItemArray: compactResponseInputItems(req.Messages),
		},
	}
	if strings.TrimSpace(req.Instructions) != "" {
		params.Instructions = param.NewOpt(req.Instructions)
	}
	if req.PromptCacheKey != "" {
		params.PromptCacheKey = param.NewOpt(req.PromptCacheKey)
	}
	return params
}

func (o *OpenAI) responseParams(req Request) responses.ResponseNewParams {
	input := responseInputItems(req.Messages)
	if len(req.InputItems) > 0 {
		input = providerInputItems(req.InputItems)
	}
	params := responses.ResponseNewParams{
		Model: responses.ResponsesModel(o.model),
		Input: responses.ResponseNewParamsInputUnion{
			OfInputItemList: input,
		},
		Reasoning: shared.ReasoningParam{
			Effort:  shared.ReasoningEffort(o.reasoning),
			Summary: shared.ReasoningSummaryAuto,
		},
		Store: param.NewOpt(req.Store),
		Tools: openAITools(req.Tools),
	}
	if req.PreviousResponseID != "" {
		params.PreviousResponseID = param.NewOpt(req.PreviousResponseID)
	}
	if req.PromptCacheKey != "" {
		params.PromptCacheKey = param.NewOpt(req.PromptCacheKey)
	}
	if len(req.Tools) > 0 {
		params.ParallelToolCalls = param.NewOpt(true)
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
	if req.Compaction.Enabled && req.Compaction.CompactThreshold > 0 {
		params.ContextManagement = []responses.ResponseNewParamsContextManagement{{
			Type:             "compaction",
			CompactThreshold: param.NewOpt(int64(req.Compaction.CompactThreshold)),
		}}
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
	if len(specs) == 0 {
		return nil
	}
	namespaceTools := make([]responses.NamespaceToolToolUnionParam, 0, len(specs))
	hasDeferredTool := false
	for _, spec := range specs {
		schema := normalizeToolSchema(spec.Schema)
		function := responses.NamespaceToolToolFunctionParam{
			Name:        spec.Name,
			Description: param.NewOpt(spec.Description),
			Parameters:  schema,
			Strict:      param.NewOpt(false),
		}
		if openAIToolShouldDefer(spec.Name) {
			function.DeferLoading = param.NewOpt(true)
			hasDeferredTool = true
		}
		namespaceTools = append(namespaceTools, responses.NamespaceToolToolUnionParam{
			OfFunction: &function,
		})
	}
	tools := []responses.ToolUnionParam{
		responses.ToolParamOfNamespace("Gode coding-agent tools for reading files, searching, editing, and inspecting the current workspace.", "gode", namespaceTools),
	}
	if hasDeferredTool {
		tools = append(tools, responses.ToolUnionParam{OfToolSearch: &responses.ToolSearchToolParam{Execution: responses.ToolSearchToolExecutionServer}})
	}
	return tools
}

func openAIToolShouldDefer(name string) bool {
	if strings.HasPrefix(name, "memory_") {
		return false
	}
	_, eager := openAIEagerToolNames[name]
	return !eager
}

var openAIEagerToolNames = map[string]struct{}{
	"apply_patch":        {},
	"create_goal":        {},
	"download":           {},
	"edit":               {},
	"get_goal":           {},
	"git_diff":           {},
	"git_status":         {},
	"glob":               {},
	"grep":               {},
	"list_files":         {},
	"list_mcp_prompts":   {},
	"list_mcp_resources": {},
	"lsp_diagnostics":    {},
	"lsp_restart":        {},
	"multi_edit":         {},
	"read_file":          {},
	"read_mcp_resource":  {},
	"run_mcp_prompt":     {},
	"search_files":       {},
	"shell":              {},
	"subagent":           {},
	"todo_update":        {},
	"update_goal":        {},
	"write_file":         {},
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
