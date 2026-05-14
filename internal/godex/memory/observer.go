package memory

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"

	"github.com/pandelisz/gode/internal/godex/provider"
)

const MaxObservationMessages = 40

type ObservationRequest struct {
	SessionID string
	RunID     string
	Messages  []provider.Message
}

type Observer struct {
	service     *Service
	provider    provider.Provider
	maxMessages int
}

func NewObserver(service *Service, provider provider.Provider) *Observer {
	return &Observer{
		service:     service,
		provider:    provider,
		maxMessages: MaxObservationMessages,
	}
}

func (o *Observer) Enabled() bool {
	return o != nil &&
		o.service != nil &&
		o.provider != nil &&
		o.service.cfg.Enabled &&
		o.service.cfg.AutoObserve
}

func (o *Observer) Observe(ctx context.Context, req ObservationRequest) error {
	if !o.Enabled() {
		return nil
	}
	providerReq := provider.Request{
		SessionID:    req.SessionID,
		RunID:        req.RunID,
		Instructions: observerInstructions,
		Messages:     boundedObservationMessages(req.Messages, o.maxMessages),
		Store:        false,
		Tools:        []provider.ToolSpec{observerSaveMemoryTool()},
	}
	events, errs := o.provider.Stream(ctx, providerReq)
	for events != nil || errs != nil {
		select {
		case ev, ok := <-events:
			if !ok {
				events = nil
				continue
			}
			if ev.Kind != provider.EventToolCall || ev.ToolRequest == nil {
				continue
			}
			if err := o.handleToolCall(ctx, req, ev.ToolRequest); err != nil {
				return err
			}
		case err, ok := <-errs:
			if !ok {
				errs = nil
				continue
			}
			if err != nil {
				return err
			}
		case <-ctx.Done():
			return ctx.Err()
		}
	}
	return nil
}

func (o *Observer) handleToolCall(ctx context.Context, req ObservationRequest, toolRequest *provider.ToolRequest) error {
	if toolRequest.Name != "memory_save" {
		return nil
	}
	content := observerToolContent(toolRequest)
	if strings.TrimSpace(content) == "" {
		return errors.New("observer memory content is required")
	}
	_, err := o.service.SaveWithMetadata(ctx, content, "observer", map[string]string{
		"session_id": req.SessionID,
		"run_id":     req.RunID,
	})
	if err != nil {
		return fmt.Errorf("save observed memory: %w", err)
	}
	return nil
}

func boundedObservationMessages(messages []provider.Message, limit int) []provider.Message {
	if limit <= 0 {
		limit = MaxObservationMessages
	}
	if len(messages) > limit {
		messages = messages[len(messages)-limit:]
	}
	return append([]provider.Message(nil), messages...)
}

func observerSaveMemoryTool() provider.ToolSpec {
	return provider.ToolSpec{
		Name:        "memory_save",
		Description: "Save a stable, reusable workspace memory discovered from the recent session.",
		Schema:      objectSchema("content"),
	}
}

func observerToolContent(toolRequest *provider.ToolRequest) string {
	if toolRequest == nil {
		return ""
	}
	if content := stringInput(toolRequest.Input, "content"); content != "" {
		return content
	}
	if strings.TrimSpace(toolRequest.Arguments) == "" {
		return ""
	}
	var args map[string]any
	if err := json.Unmarshal([]byte(toolRequest.Arguments), &args); err != nil {
		return ""
	}
	return stringInput(args, "content")
}

const observerInstructions = `Review the recent agent conversation and tool results.
Extract only durable, reusable facts about this workspace that will help future coding-agent sessions.
Call memory_save once per useful fact using concise content.
Do not save transient task state, guesses, secrets, credentials, or user-private data.
If there are no stable facts worth saving, finish without calling any tool.`
