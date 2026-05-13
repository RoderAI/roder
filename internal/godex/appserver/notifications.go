package appserver

import (
	"context"
	"time"

	"github.com/pandelisz/gode/internal/godex/eventbus"
)

func (s *Server) startEventBridge(ctx context.Context) {
	if s.app == nil || s.app.Bus == nil {
		return
	}
	events := s.app.Bus.Subscribe(ctx, eventbus.Filter{Kinds: []eventbus.Kind{
		eventbus.KindMCPStateChanged,
		eventbus.KindLSPStateChanged,
		eventbus.KindLSPDiagnostics,
		eventbus.KindPermissionRequested,
		eventbus.KindPermissionResponded,
		eventbus.KindGoalUpdated,
		eventbus.KindGoalCleared,
		eventbus.KindGoalLimited,
	}})
	go func() {
		for ev := range events {
			method := notificationMethod(ev.Kind)
			if method == "" {
				continue
			}
			if ev.Kind == eventbus.KindGoalUpdated || ev.Kind == eventbus.KindGoalLimited || ev.Kind == eventbus.KindGoalCleared {
				s.notifyThread(context.Background(), ev.SessionID, method, eventNotificationParams(ev))
				continue
			}
			s.notifyAll(context.Background(), method, eventNotificationParams(ev))
		}
	}()
}

func (s *Server) notifyThread(ctx context.Context, threadID, method string, params any) {
	for _, conn := range s.subscribers(threadID) {
		_ = conn.sendNotification(ctx, method, params)
	}
}

func (s *Server) subscribers(threadID string) []*Connection {
	s.mu.RLock()
	conns := make([]*Connection, 0, len(s.conns))
	for conn := range s.conns {
		if conn.isSubscribed(threadID) {
			conns = append(conns, conn)
		}
	}
	s.mu.RUnlock()
	return conns
}

func (s *Server) notifyAll(ctx context.Context, method string, params any) {
	s.mu.RLock()
	conns := make([]*Connection, 0, len(s.conns))
	for conn := range s.conns {
		if conn.isInitialized() {
			conns = append(conns, conn)
		}
	}
	s.mu.RUnlock()
	for _, conn := range conns {
		_ = conn.sendNotification(ctx, method, params)
	}
}

func notificationMethod(kind eventbus.Kind) string {
	switch kind {
	case eventbus.KindMCPStateChanged:
		return "mcp/state/changed"
	case eventbus.KindLSPStateChanged:
		return "lsp/state/changed"
	case eventbus.KindLSPDiagnostics:
		return "lsp/diagnostics/changed"
	case eventbus.KindPermissionRequested:
		return "permission/requested"
	case eventbus.KindPermissionResponded:
		return "permission/responded"
	case eventbus.KindGoalUpdated, eventbus.KindGoalLimited:
		return "thread/goal/updated"
	case eventbus.KindGoalCleared:
		return "thread/goal/cleared"
	default:
		return ""
	}
}

func eventNotificationParams(ev eventbus.Event) map[string]any {
	out := map[string]any{
		"eventId": ev.ID,
		"seq":     ev.Seq,
		"time":    ev.Time.Format(time.RFC3339Nano),
		"payload": ev.Payload,
	}
	if ev.SessionID != "" {
		out["sessionId"] = ev.SessionID
	}
	if ev.RunID != "" {
		out["runId"] = ev.RunID
	}
	if ev.CorrelationID != "" {
		out["correlationId"] = ev.CorrelationID
	}
	return out
}
