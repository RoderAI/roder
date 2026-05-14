package remote

import "strings"

func SecurityWarning(state State) string {
	if !state.Running {
		return ""
	}
	for _, remoteURL := range state.URLs {
		if strings.HasPrefix(remoteURL, "ws://127.") || strings.HasPrefix(remoteURL, "ws://[::1]") {
			continue
		}
		return "warning: LAN websocket uses bearer auth without TLS; prefer Tailscale on shared networks"
	}
	return ""
}
