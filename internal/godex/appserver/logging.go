package appserver

import (
	"fmt"
	"sort"
	"strings"
	"time"
)

func (s *Server) logRemote(format string, args ...any) {
	if s == nil || s.options.Log == nil || !s.options.Remote.Enabled {
		return
	}
	message := fmt.Sprintf(format, args...)
	fmt.Fprintf(s.options.Log, "remote %s %s\n", time.Now().Format(time.RFC3339), message)
}

func remoteRequestSummary(r remoteRequestInfo) string {
	parts := []string{}
	if r.RemoteAddr != "" {
		parts = append(parts, "addr="+r.RemoteAddr)
	}
	if r.Path != "" {
		parts = append(parts, "path="+r.Path)
	}
	if r.Origin != "" {
		parts = append(parts, "origin="+r.Origin)
	}
	if r.AuthHeader {
		parts = append(parts, "auth=header")
	}
	if r.BearerSubprotocol {
		parts = append(parts, "auth=subprotocol")
	}
	if len(r.RequestedSubprotocols) > 0 {
		parts = append(parts, "subprotocols="+strings.Join(r.RequestedSubprotocols, ","))
	}
	sort.Strings(parts)
	return strings.Join(parts, " ")
}
