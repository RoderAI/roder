package permission

import (
	"context"
	"errors"
	"strings"
	"sync"

	"github.com/google/uuid"
	"github.com/pandelisz/gode/internal/godex/eventbus"
)

type Action string

const (
	ActionRead    Action = "read"
	ActionWrite   Action = "write"
	ActionExecute Action = "execute"
	ActionNetwork Action = "network"
)

type Request struct {
	SessionID   string
	RunID       string
	Tool        string
	Action      Action
	Path        string
	Description string
	Input       map[string]any
}

type Result struct {
	Approved        bool
	AllowForSession bool
	Reason          string
}

type Service struct {
	bus          *eventbus.Bus
	allowedTools map[string]struct{}
	skipRequests bool
	mu           sync.Mutex
	grants       map[grantKey]struct{}
}

type Option func(*Service)

func WithEventBus(bus *eventbus.Bus) Option {
	return func(s *Service) {
		s.bus = bus
	}
}

func WithAllowedTools(tools ...string) Option {
	return func(s *Service) {
		for _, tool := range tools {
			tool = strings.TrimSpace(tool)
			if tool != "" {
				s.allowedTools[tool] = struct{}{}
			}
		}
	}
}

func WithSkipRequests(skip bool) Option {
	return func(s *Service) {
		s.skipRequests = skip
	}
}

func New(opts ...Option) *Service {
	s := &Service{
		allowedTools: map[string]struct{}{},
		grants:       map[grantKey]struct{}{},
	}
	for _, opt := range opts {
		opt(s)
	}
	return s
}

func (s *Service) Authorize(ctx context.Context, req Request) (Result, error) {
	if err := ctx.Err(); err != nil {
		return Result{}, err
	}
	req = normalizeRequest(req)
	if req.Tool == "" {
		return Result{}, errors.New("permission tool is required")
	}
	if s.skipRequests {
		return Result{Approved: true, Reason: "skip_requests"}, nil
	}
	if _, ok := s.allowedTools[req.Tool]; ok {
		return Result{Approved: true, Reason: "allowed_tool"}, nil
	}
	key := grantKeyFor(req)
	s.mu.Lock()
	_, granted := s.grants[key]
	s.mu.Unlock()
	if granted {
		return Result{Approved: true, AllowForSession: true, Reason: "session_grant"}, nil
	}
	if s.bus == nil {
		return Result{}, errors.New("permission event bus is required")
	}

	correlationID := uuid.NewString()
	responses := s.bus.Subscribe(ctx, eventbus.Filter{
		CorrelationID: correlationID,
		Kinds:         []eventbus.Kind{eventbus.KindPermissionResponded},
	})
	s.bus.Publish(ctx, eventbus.Event{
		Kind:          eventbus.KindPermissionRequested,
		Source:        eventbus.SourceTool,
		SessionID:     req.SessionID,
		RunID:         req.RunID,
		CorrelationID: correlationID,
		Payload: map[string]any{
			"tool":              req.Tool,
			"action":            string(req.Action),
			"path":              req.Path,
			"description":       req.Description,
			"input":             req.Input,
			"allow_for_session": true,
		},
	})

	var response eventbus.Event
	select {
	case ev, ok := <-responses:
		if !ok {
			return Result{}, eventbus.ErrClosed
		}
		response = ev
	case <-ctx.Done():
		return Result{}, ctx.Err()
	}
	var payload struct {
		Approved        bool   `json:"approved"`
		AllowForSession bool   `json:"allow_for_session"`
		Reason          string `json:"reason"`
	}
	if err := response.DecodePayload(&payload); err != nil {
		return Result{}, err
	}
	result := Result{Approved: payload.Approved, AllowForSession: payload.AllowForSession, Reason: payload.Reason}
	if result.Approved && result.AllowForSession {
		s.mu.Lock()
		s.grants[key] = struct{}{}
		s.mu.Unlock()
	}
	return result, nil
}

type grantKey struct {
	sessionID string
	tool      string
	action    Action
	path      string
}

func grantKeyFor(req Request) grantKey {
	return grantKey{
		sessionID: req.SessionID,
		tool:      req.Tool,
		action:    req.Action,
		path:      req.Path,
	}
}

func normalizeRequest(req Request) Request {
	req.Tool = strings.TrimSpace(req.Tool)
	req.Path = strings.TrimSpace(req.Path)
	if req.Action == "" {
		req.Action = ActionRead
	}
	if req.Input == nil {
		req.Input = map[string]any{}
	}
	return req
}
