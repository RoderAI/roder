package appserver

import (
	"context"
	"net/http"
	"strings"

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
	return remoteRequestInfoFromRequest(r).Payload()
}

type remoteRequestInfo struct {
	RemoteAddr            string
	Path                  string
	Origin                string
	AuthHeader            bool
	BearerSubprotocol     bool
	RequestedSubprotocols []string
}

func remoteRequestInfoFromRequest(r *http.Request) remoteRequestInfo {
	info := remoteRequestInfo{}
	if r == nil {
		return info
	}
	info.RemoteAddr = r.RemoteAddr
	info.Path = r.URL.Path
	info.Origin = r.Header.Get("Origin")
	info.AuthHeader = strings.TrimSpace(r.Header.Get("Authorization")) != ""
	for _, value := range r.Header.Values("Sec-WebSocket-Protocol") {
		for _, part := range strings.Split(value, ",") {
			part = strings.TrimSpace(part)
			if part == "" {
				continue
			}
			if strings.HasPrefix(part, "bearer.") {
				info.BearerSubprotocol = true
				info.RequestedSubprotocols = append(info.RequestedSubprotocols, "bearer.<redacted>")
				continue
			}
			info.RequestedSubprotocols = append(info.RequestedSubprotocols, part)
		}
	}
	return info
}

func (r remoteRequestInfo) Payload() map[string]any {
	payload := map[string]any{}
	if r.RemoteAddr != "" {
		payload["remote_addr"] = r.RemoteAddr
	}
	if r.Path != "" {
		payload["path"] = r.Path
	}
	if r.Origin != "" {
		payload["origin"] = r.Origin
	}
	if r.AuthHeader {
		payload["auth_header"] = true
	}
	if r.BearerSubprotocol {
		payload["bearer_subprotocol"] = true
	}
	if len(r.RequestedSubprotocols) > 0 {
		payload["requested_subprotocols"] = r.RequestedSubprotocols
	}
	return payload
}
