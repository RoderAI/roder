package provider

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"strings"
)

type ChatCompletionsConfig struct {
	Model      string
	BaseURL    string
	APIKey     string
	Headers    map[string]string
	HeaderEnv  map[string]string
	HTTPClient *http.Client
}

type ChatCompletions struct {
	model     string
	baseURL   string
	apiKey    string
	headers   map[string]string
	headerEnv map[string]string
	client    *http.Client
}

type chatCompletionRequest struct {
	Model    string        `json:"model"`
	Messages []ChatMessage `json:"messages"`
	Tools    []ChatTool    `json:"tools,omitempty"`
	Stream   bool          `json:"stream"`
}

func NewChatCompletionsWithConfig(cfg ChatCompletionsConfig) *ChatCompletions {
	if cfg.Model == "" {
		cfg.Model = "gpt-5.5"
	}
	if cfg.BaseURL == "" {
		cfg.BaseURL = "https://api.openai.com/v1"
	}
	client := cfg.HTTPClient
	if client == nil {
		client = http.DefaultClient
	}
	return &ChatCompletions{
		model:     cfg.Model,
		baseURL:   strings.TrimRight(cfg.BaseURL, "/"),
		apiKey:    cfg.APIKey,
		headers:   cloneStringMap(cfg.Headers),
		headerEnv: cloneStringMap(cfg.HeaderEnv),
		client:    client,
	}
}

func (c *ChatCompletions) Name() string {
	return "chat_completions"
}

func (c *ChatCompletions) Stream(ctx context.Context, req Request) (<-chan Event, <-chan error) {
	events := make(chan Event)
	errs := make(chan error, 1)
	go func() {
		defer close(events)
		defer close(errs)
		input, err := ChatInputFromResponsesItems(chatInputItems(req), req.Tools)
		if err != nil {
			errs <- err
			return
		}
		payload := chatCompletionRequest{
			Model:    c.model,
			Messages: input.Messages,
			Tools:    input.Tools,
			Stream:   true,
		}
		body, err := json.Marshal(payload)
		if err != nil {
			errs <- err
			return
		}
		httpReq, err := http.NewRequestWithContext(ctx, http.MethodPost, c.endpoint(), bytes.NewReader(body))
		if err != nil {
			errs <- err
			return
		}
		httpReq.Header.Set("Content-Type", "application/json")
		httpReq.Header.Set("Accept", "text/event-stream")
		if c.apiKey != "" {
			httpReq.Header.Set("Authorization", "Bearer "+c.apiKey)
		}
		for key, value := range c.headers {
			if strings.TrimSpace(key) != "" {
				httpReq.Header.Set(key, value)
			}
		}
		for key, envName := range c.headerEnv {
			if strings.TrimSpace(key) != "" && strings.TrimSpace(envName) != "" {
				httpReq.Header.Set(key, os.Getenv(envName))
			}
		}
		resp, err := c.client.Do(httpReq)
		if err != nil {
			errs <- err
			return
		}
		defer resp.Body.Close()
		if resp.StatusCode < 200 || resp.StatusCode > 299 {
			errs <- chatHTTPError(resp)
			return
		}
		state := newChatStreamState()
		scanner := bufio.NewScanner(resp.Body)
		for scanner.Scan() {
			line := scanner.Bytes()
			emitted, err := state.HandleChatSSELine(line)
			if err != nil {
				errs <- err
				return
			}
			for _, ev := range emitted {
				select {
				case <-ctx.Done():
					errs <- ctx.Err()
					return
				case events <- ev:
				}
			}
			if state.Done() {
				return
			}
		}
		if err := scanner.Err(); err != nil {
			errs <- err
			return
		}
		if !state.Done() {
			select {
			case <-ctx.Done():
				errs <- ctx.Err()
			case events <- state.CompletedEvent():
			}
		}
	}()
	return events, errs
}

func (c *ChatCompletions) endpoint() string {
	return strings.TrimRight(c.baseURL, "/") + "/chat/completions"
}

func chatHTTPError(resp *http.Response) error {
	data, _ := io.ReadAll(io.LimitReader(resp.Body, 4096))
	detail := strings.TrimSpace(string(data))
	if detail == "" {
		detail = resp.Status
	}
	return &ProviderError{Message: fmt.Sprintf("Chat Completions request failed: %s: %s", resp.Status, detail)}
}

func chatInputItems(req Request) []Item {
	if len(req.InputItems) > 0 {
		return req.InputItems
	}
	items := make([]Item, 0, len(req.Messages))
	for _, msg := range req.Messages {
		switch {
		case len(msg.RawJSON) > 0:
			items = append(items, Item{Kind: ItemRaw, RawJSON: append(json.RawMessage(nil), msg.RawJSON...)})
		case msg.ToolCallID != "" && msg.Role == RoleTool:
			items = append(items, Item{Kind: ItemFunctionOut, Role: string(RoleTool), ToolCallID: msg.ToolCallID, Text: msg.Content})
		case msg.ToolCallID != "":
			items = append(items, Item{Kind: ItemFunctionCall, Role: string(RoleAssistant), ToolName: msg.ToolName, ToolCallID: msg.ToolCallID, Text: msg.ToolArguments})
		default:
			items = append(items, Item{Kind: ItemMessage, Role: string(msg.Role), Text: msg.Content, Images: append([]Image(nil), msg.Images...)})
		}
	}
	return items
}
