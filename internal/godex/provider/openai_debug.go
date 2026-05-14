package provider

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"strings"

	openai "github.com/openai/openai-go/v3"
	"github.com/pandelisz/gode/internal/godex/contextwindow"
)

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
	estimate := requestTokenEstimate(req, o.model)
	lines = append(lines,
		"model: "+o.model,
		"reasoning: "+o.reasoning,
		fmt.Sprintf("messages: %d", len(req.Messages)),
		fmt.Sprintf("input_items: %d", len(req.InputItems)),
		fmt.Sprintf("input_chars: %d", requestInputChars(req)),
		fmt.Sprintf("estimated_input_tokens: %d", estimate.Tokens),
		fmt.Sprintf("tools: %d", len(req.Tools)),
	)
	if req.Compaction.Enabled {
		lines = append(lines,
			fmt.Sprintf("compaction_threshold: %d", req.Compaction.CompactThreshold),
			fmt.Sprintf("context_window: %d", req.Compaction.ContextWindow),
		)
	}
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

func requestTokenEstimate(req Request, model string) contextwindow.TokenEstimate {
	return contextwindow.EstimateRequest(contextwindow.Request{
		Model:          model,
		Instructions:   req.Instructions,
		ResponseFormat: req.ResponseFormat,
		Messages:       contextMessages(req.Messages),
		InputItems:     contextItems(req.InputItems),
		Tools:          contextTools(req.Tools),
	}, contextwindow.ForModel(model))
}

func requestInputChars(req Request) int {
	total := len(req.Instructions) + len(req.ResponseFormat)
	if len(req.InputItems) > 0 {
		for _, item := range req.InputItems {
			total += len(item.Text) + len(item.RawJSON) + imageChars(item.Images)
		}
	} else {
		for _, msg := range req.Messages {
			total += len(msg.Content) + len(msg.RawJSON) + len(msg.ToolArguments) + imageChars(msg.Images)
		}
	}
	for _, tool := range req.Tools {
		total += len(tool.Name) + len(tool.Description)
		if tool.Schema != nil {
			if data, err := json.Marshal(tool.Schema); err == nil {
				total += len(data)
			}
		}
	}
	return total
}

func imageChars(images []Image) int {
	total := 0
	for _, image := range images {
		total += len(image.URL) + len(image.Detail)
	}
	return total
}

func contextMessages(messages []Message) []contextwindow.Message {
	out := make([]contextwindow.Message, 0, len(messages))
	for _, msg := range messages {
		out = append(out, contextwindow.Message{
			Role:          string(msg.Role),
			Content:       msg.Content,
			Phase:         msg.Phase,
			Images:        contextImages(msg.Images),
			ToolCallID:    msg.ToolCallID,
			ToolName:      msg.ToolName,
			ToolArguments: msg.ToolArguments,
			RawJSON:       msg.RawJSON,
		})
	}
	return out
}

func contextItems(items []Item) []contextwindow.Item {
	out := make([]contextwindow.Item, 0, len(items))
	for _, item := range items {
		out = append(out, contextwindow.Item{
			Kind:       string(item.Kind),
			Role:       item.Role,
			Phase:      item.Phase,
			ToolName:   item.ToolName,
			ToolCallID: item.ToolCallID,
			Text:       item.Text,
			Images:     contextImages(item.Images),
			RawJSON:    item.RawJSON,
		})
	}
	return out
}

func contextTools(tools []ToolSpec) []contextwindow.ToolSpec {
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

func contextImages(images []Image) []contextwindow.Image {
	out := make([]contextwindow.Image, 0, len(images))
	for _, image := range images {
		out = append(out, contextwindow.Image{URL: image.URL, Detail: image.Detail})
	}
	return out
}
