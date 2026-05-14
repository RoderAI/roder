package appserver

import (
	"context"
	"net/http"

	"github.com/pandelisz/gode/internal/godex/eventbus"
)

const (
	KindRemoteServerStarted      eventbus.Kind = "remote.server_started"
	KindRemoteServerStopped      eventbus.Kind = "remote.server_stopped"
	KindRemoteClientConnected    eventbus.Kind = "remote.client_connected"
	KindRemoteClientDisconnected eventbus.Kind = "remote.client_disconnected"
	KindRemoteAuthFailed         eventbus.Kind = "remote.auth_failed"
)

func (s *Server) publishRemoteEvent(ctx context.Context, kind eventbus.Kind, payload map[string]any) {
	if s == nil || s.app == nil || s.app.Bus == nil {
		return
	}
	if payload == nil {
		payload = map[string]any{}
	}
	payload["remote"] = true
	s.app.Bus.Publish(ctx, eventbus.NewEvent(kind, eventbus.SourceSystem, payload))
}

func redactedRemoteRequestPayload(r *http.Request) map[string]any {
	payload := map[string]any{}
	if r == nil {
		return payload
	}
	payload["remote_addr"] = r.RemoteAddr
	payload["path"] = r.URL.Path
	if origin := r.Header.Get("Origin"); origin != "" {
		payload["origin"] = origin
	}
	return payload
}
