package appserver

import (
	"context"
	"encoding/json"
	"strings"

	"github.com/pandelisz/gode/internal/godex/eventbus"
)

type permissionRespondParams struct {
	CorrelationID        string `json:"correlationId"`
	Approved             bool   `json:"approved"`
	AllowForSession      bool   `json:"allowForSession,omitempty"`
	AllowForSessionSnake bool   `json:"allow_for_session,omitempty"`
	Reason               string `json:"reason,omitempty"`
}

func (s *Server) handlePermissionRespond(ctx context.Context, raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[permissionRespondParams](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	params.CorrelationID = strings.TrimSpace(params.CorrelationID)
	if params.CorrelationID == "" {
		return nil, rpcError(errorInvalidParams, "correlationId is required")
	}
	if s.app == nil || s.app.Bus == nil {
		return nil, rpcError(errorInternal, "event bus is not available")
	}
	s.app.Bus.Publish(ctx, eventbus.Event{
		Kind:          eventbus.KindPermissionResponded,
		Source:        eventbus.SourceSystem,
		CorrelationID: params.CorrelationID,
		Payload: map[string]any{
			"approved":          params.Approved,
			"allow_for_session": params.AllowForSession || params.AllowForSessionSnake,
			"reason":            params.Reason,
		},
	})
	return map[string]any{}, nil
}
