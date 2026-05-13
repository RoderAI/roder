package appserver

import "context"

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
