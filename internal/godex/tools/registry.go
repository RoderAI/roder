package tools

import (
	"context"
	"errors"
	"fmt"
	"sort"
	"time"

	"github.com/google/uuid"
	"github.com/pandelisz/gode/internal/godex/eventbus"
)

type Call struct {
	ID        string
	Name      string
	Input     map[string]any
	SessionID string
	RunID     string
}

type Result struct {
	Text string
	Data any
}

type Tool struct {
	Name        string
	Description string
	Schema      map[string]any
	ReadOnly    bool
	Run         func(context.Context, Call) (Result, error)
}

type Registry struct {
	tools       map[string]Tool
	bus         *eventbus.Bus
	autoApprove bool
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

func NewRegistry(opts ...Option) *Registry {
	r := &Registry{
		tools:       make(map[string]Tool),
		autoApprove: true,
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

	if !tool.ReadOnly && !r.autoApprove {
		approved, err := r.requestPermission(ctx, tool, call)
		if err != nil {
			return Result{}, err
		}
		if !approved {
			return Result{}, errors.New("permission denied")
		}
	}

	r.publish(ctx, eventbus.KindToolStarted, call, map[string]any{"tool": call.Name, "tool_call_id": call.ID})
	result, err := tool.Run(ctx, call)
	if err != nil {
		r.publish(ctx, eventbus.KindToolFailed, call, map[string]any{"tool": call.Name, "tool_call_id": call.ID, "error": err.Error()})
		return Result{}, err
	}
	r.publish(ctx, eventbus.KindToolCompleted, call, map[string]any{"tool": call.Name, "tool_call_id": call.ID, "text": result.Text})
	return result, nil
}

func (r *Registry) requestPermission(ctx context.Context, tool Tool, call Call) (bool, error) {
	if r.bus == nil {
		return false, errors.New("permission required but event bus is nil")
	}
	correlationID := uuid.NewString()
	r.bus.Publish(ctx, eventbus.Event{
		Source:        eventbus.SourceTool,
		Kind:          eventbus.KindPermissionRequested,
		SessionID:     call.SessionID,
		RunID:         call.RunID,
		CorrelationID: correlationID,
		Payload: map[string]any{
			"tool":         tool.Name,
			"tool_call_id": call.ID,
			"description":  tool.Description,
			"input":        call.Input,
		},
	})

	awaitCtx, cancel := context.WithTimeout(ctx, 24*time.Hour)
	defer cancel()
	response, err := r.bus.Await(awaitCtx, eventbus.Filter{
		CorrelationID: correlationID,
		Kinds:         []eventbus.Kind{eventbus.KindPermissionResponded},
	})
	if err != nil {
		return false, err
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
