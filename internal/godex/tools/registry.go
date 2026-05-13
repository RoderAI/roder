package tools

import (
	"context"
	"errors"
	"fmt"
	"sort"
	"strings"
	"time"

	"github.com/google/uuid"
	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/hooks"
	"github.com/pandelisz/gode/internal/godex/permission"
)

type Call struct {
	ID        string
	Name      string
	Input     map[string]any
	SessionID string
	RunID     string
}

type Result struct {
	Text  string
	Data  any
	Error string
}

type Tool struct {
	Name           string
	Description    string
	Schema         map[string]any
	ReadOnly       bool
	SkipPermission bool
	Action         permission.Action
	PathFromInput  func(map[string]any) string
	Network        bool
	Run            func(context.Context, Call) (Result, error)
}

type Registry struct {
	tools        map[string]Tool
	bus          *eventbus.Bus
	autoApprove  bool
	permissions  *permission.Service
	hooks        *hooks.Runner
	workspace    string
	allowedTools map[string]struct{}
}

type Option func(*Registry)

func WithEventBus(bus *eventbus.Bus) Option {
	return func(r *Registry) {
		r.bus = bus
	}
}

func WithAutoApprove(autoApprove bool) Option {
	return func(r *Registry) {
		r.autoApprove = autoApprove
	}
}

func WithPermissionService(service *permission.Service) Option {
	return func(r *Registry) {
		r.permissions = service
	}
}

func WithHookRunner(runner *hooks.Runner) Option {
	return func(r *Registry) {
		r.hooks = runner
	}
}

func WithWorkspace(workspace string) Option {
	return func(r *Registry) {
		r.workspace = workspace
	}
}

func WithAllowedTools(tools ...string) Option {
	return func(r *Registry) {
		for _, tool := range tools {
			tool = strings.TrimSpace(tool)
			if tool != "" {
				r.allowedTools[tool] = struct{}{}
			}
		}
	}
}

func NewRegistry(opts ...Option) *Registry {
	r := &Registry{
		tools:        make(map[string]Tool),
		autoApprove:  true,
		allowedTools: make(map[string]struct{}),
	}
	for _, opt := range opts {
		opt(r)
	}
	return r
}

func (r *Registry) Register(tool Tool) {
	r.tools[tool.Name] = tool
}

func (r *Registry) Specs() []Spec {
	specs := make([]Spec, 0, len(r.tools))
	for _, tool := range r.tools {
		specs = append(specs, Spec{Name: tool.Name, Description: tool.Description, Schema: tool.Schema})
	}
	sort.Slice(specs, func(i, j int) bool { return specs[i].Name < specs[j].Name })
	return specs
}

func (r *Registry) Run(ctx context.Context, call Call) (Result, error) {
	tool, ok := r.tools[call.Name]
	if !ok {
		return Result{}, fmt.Errorf("tool %q not found", call.Name)
	}
	if tool.Run == nil {
		return Result{}, fmt.Errorf("tool %q has no runner", call.Name)
	}
	if call.ID == "" {
		call.ID = uuid.NewString()
	}

	hookResult, err := r.runHooks(ctx, tool, &call)
	if err != nil {
		return Result{}, err
	}
	if hookResult.Decision == hooks.DecisionDeny || hookResult.Decision == hooks.DecisionHalt {
		result := failedResult(Result{}, fmt.Errorf("tool %s blocked by hook: %s", call.Name, hookResult.Decision))
		r.publish(ctx, eventbus.KindToolFailed, call, toolPayload(tool, call, map[string]any{"error": result.Error, "text": result.Text, "hook_decision": string(hookResult.Decision), "hook_context": hookResult.Context, "hook_warnings": hookResult.Warnings}))
		return result, nil
	}

	if err := r.authorize(ctx, tool, call, hookResult); err != nil {
		result := failedResult(Result{}, err)
		r.publish(ctx, eventbus.KindToolFailed, call, toolPayload(tool, call, map[string]any{"error": result.Error, "text": result.Text}))
		return result, nil
	}

	r.publish(ctx, eventbus.KindToolStarted, call, toolPayload(tool, call, map[string]any{"hook_decision": string(hookResult.Decision), "hook_context": hookResult.Context, "hook_warnings": hookResult.Warnings}))
	result, err := tool.Run(ctx, call)
	if err != nil {
		result = failedResult(result, err)
		r.publish(ctx, eventbus.KindToolFailed, call, toolPayload(tool, call, map[string]any{"error": result.Error, "text": result.Text}))
		return result, nil
	}
	r.publish(ctx, eventbus.KindToolCompleted, call, toolPayload(tool, call, map[string]any{"text": result.Text}))
	return result, nil
}

func (r *Registry) runHooks(ctx context.Context, tool Tool, call *Call) (hooks.HookResult, error) {
	if r.hooks == nil || strings.HasPrefix(call.Name, "mcp.") || strings.HasPrefix(call.Name, "mcp_") {
		return hooks.HookResult{Decision: hooks.DecisionNone, UpdatedInput: call.Input}, nil
	}
	result, err := r.hooks.Run(ctx, hooks.HookInput{
		Tool:      call.Name,
		SessionID: call.SessionID,
		Workspace: r.workspace,
		Input:     call.Input,
	})
	if err != nil {
		return hooks.HookResult{}, err
	}
	if result.UpdatedInput != nil {
		call.Input = result.UpdatedInput
	}
	return result, nil
}

func (r *Registry) authorize(ctx context.Context, tool Tool, call Call, hookResult hooks.HookResult) error {
	if tool.ReadOnly || tool.SkipPermission || r.autoApprove || hookResult.Decision == hooks.DecisionAllow {
		return nil
	}
	if _, ok := r.allowedTools[call.Name]; ok {
		return nil
	}
	action := toolAction(tool)
	path := toolPath(tool, call.Input)
	if r.permissions != nil {
		result, err := r.permissions.Authorize(ctx, permission.Request{
			SessionID:   call.SessionID,
			RunID:       call.RunID,
			Tool:        call.Name,
			Action:      action,
			Path:        path,
			Description: tool.Description,
			Input:       call.Input,
		})
		if err != nil {
			return err
		}
		if !result.Approved {
			if result.Reason != "" {
				return fmt.Errorf("permission denied: %s", result.Reason)
			}
			return errors.New("permission denied")
		}
		return nil
	}
	approved, err := r.requestPermission(ctx, tool, call)
	if err != nil {
		return err
	}
	if !approved {
		return errors.New("permission denied")
	}
	return nil
}

func failedResult(result Result, err error) Result {
	message := strings.TrimSpace(err.Error())
	output := strings.TrimSpace(result.Text)
	result.Error = message
	switch {
	case message == "":
		result.Text = output
	case output == "":
		result.Text = message
	case strings.Contains(message, output):
		result.Text = message
	default:
		result.Text = message + "\n" + output
	}
	return result
}

func (r *Registry) requestPermission(ctx context.Context, tool Tool, call Call) (bool, error) {
	if r.bus == nil {
		return false, errors.New("permission required but event bus is nil")
	}
	correlationID := uuid.NewString()

	awaitCtx, cancel := context.WithTimeout(ctx, 24*time.Hour)
	defer cancel()
	responses := r.bus.Subscribe(awaitCtx, eventbus.Filter{
		CorrelationID: correlationID,
		Kinds:         []eventbus.Kind{eventbus.KindPermissionResponded},
	})

	r.bus.Publish(ctx, eventbus.Event{
		Source:        eventbus.SourceTool,
		Kind:          eventbus.KindPermissionRequested,
		SessionID:     call.SessionID,
		RunID:         call.RunID,
		CorrelationID: correlationID,
		Payload: map[string]any{
			"tool":         tool.Name,
			"tool_call_id": call.ID,
			"action":       string(toolAction(tool)),
			"path":         toolPath(tool, call.Input),
			"description":  tool.Description,
			"input":        call.Input,
		},
	})

	var response eventbus.Event
	select {
	case ev, ok := <-responses:
		if !ok {
			return false, eventbus.ErrClosed
		}
		response = ev
	case <-awaitCtx.Done():
		return false, awaitCtx.Err()
	}
	var payload struct {
		Approved bool `json:"approved"`
	}
	if err := response.DecodePayload(&payload); err != nil {
		return false, err
	}
	return payload.Approved, nil
}

func (r *Registry) publish(ctx context.Context, kind eventbus.Kind, call Call, payload any) {
	if r.bus == nil {
		return
	}
	r.bus.Publish(ctx, eventbus.Event{
		Source:    eventbus.SourceTool,
		Kind:      kind,
		SessionID: call.SessionID,
		RunID:     call.RunID,
		Payload:   payload,
	})
}

type Spec struct {
	Name        string
	Description string
	Schema      map[string]any
}

func toolAction(tool Tool) permission.Action {
	if tool.Action != "" {
		return tool.Action
	}
	if tool.Network {
		return permission.ActionNetwork
	}
	if tool.ReadOnly {
		return permission.ActionRead
	}
	return permission.ActionWrite
}

func toolPath(tool Tool, input map[string]any) string {
	if tool.PathFromInput != nil {
		return strings.TrimSpace(tool.PathFromInput(input))
	}
	for _, key := range []string{"path", "file", "target"} {
		if value, ok := input[key].(string); ok {
			return strings.TrimSpace(value)
		}
	}
	return ""
}

func toolPayload(tool Tool, call Call, extra map[string]any) map[string]any {
	payload := map[string]any{
		"tool":         call.Name,
		"tool_call_id": call.ID,
		"input":        call.Input,
		"action":       string(toolAction(tool)),
		"path":         toolPath(tool, call.Input),
	}
	for key, value := range extra {
		if value != nil {
			payload[key] = value
		}
	}
	return payload
}
