package appserver

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
	"net"
	"net/netip"
	"net/url"
	"sort"
	"strings"
)

type RemotePairingPayload struct {
	Type         string            `json:"type"`
	Name         string            `json:"name"`
	URL          string            `json:"url"`
	Headers      map[string]string `json:"headers"`
	Subprotocols []string          `json:"subprotocols"`
	Workspace    string            `json:"workspace"`
}

func BuildRemotePairingPayload(name, wsURL, token, workspace string) RemotePairingPayload {
	if name == "" {
		name = "Gode Remote"
	}
	return RemotePairingPayload{
		Type:      remoteSubprotocol,
		Name:      name,
		URL:       wsURL,
		Headers:   map[string]string{"Authorization": "Bearer " + token},
		Workspace: workspace,
		Subprotocols: []string{
			remoteSubprotocol,
			"bearer." + token,
		},
	}
}

func RemoteDeepLink(payload RemotePairingPayload) (string, error) {
	if remotePairingBearerToken(payload) == "" {
		return "", fmt.Errorf("remote pairing payload is missing bearer token")
	}
	wsURL, err := url.Parse(payload.URL)
	if err != nil {
		return "", fmt.Errorf("parse remote websocket url: %w", err)
	}
	if wsURL.Host == "" {
		return "", fmt.Errorf("remote websocket url is missing host")
	}
	data, err := json.Marshal(payload)
	if err != nil {
		return "", fmt.Errorf("marshal remote pairing payload: %w", err)
	}
	values := url.Values{}
	values.Set("payload", base64.RawURLEncoding.EncodeToString(data))
	link := url.URL{Scheme: "gode", Host: "connect", RawQuery: values.Encode()}
	return link.String(), nil
}

func DecodeRemoteDeepLink(link string) (RemotePairingPayload, error) {
	parsed, err := url.Parse(link)
	if err != nil {
		return RemotePairingPayload{}, fmt.Errorf("parse remote deep link: %w", err)
	}
	if parsed.Scheme != "gode" || parsed.Host != "connect" {
		return RemotePairingPayload{}, fmt.Errorf("remote deep link must use gode://connect")
	}
	encoded := parsed.Query().Get("payload")
	if encoded == "" {
		return RemotePairingPayload{}, fmt.Errorf("remote deep link is missing payload")
	}
	data, err := base64.RawURLEncoding.DecodeString(encoded)
	if err != nil {
		return RemotePairingPayload{}, fmt.Errorf("decode remote pairing payload: %w", err)
	}
	var payload RemotePairingPayload
	if err := json.Unmarshal(data, &payload); err != nil {
		return RemotePairingPayload{}, fmt.Errorf("unmarshal remote pairing payload: %w", err)
	}
	return payload, nil
}

func remotePairingBearerToken(payload RemotePairingPayload) string {
	const prefix = "Bearer "
	for key, value := range payload.Headers {
		if strings.EqualFold(key, "authorization") && strings.HasPrefix(value, prefix) {
			return strings.TrimSpace(strings.TrimPrefix(value, prefix))
		}
	}
	for _, subprotocol := range payload.Subprotocols {
		token, ok := strings.CutPrefix(subprotocol, "bearer.")
		if ok {
			return token
		}
	}
	return ""
}

func DiscoverRemoteConnectURLs(listenerAddress string) []string {
	addrs := localInterfaceAddrs()
	return RemoteConnectURLs(listenerAddress, addrs)
}

func RemoteConnectURLs(listenerAddress string, addrs []netip.Addr) []string {
	host, port, err := net.SplitHostPort(listenerAddress)
	if err != nil {
		return nil
	}
	if !isWildcardHost(host) {
		return []string{"ws://" + net.JoinHostPort(host, port)}
	}
	hosts := sortedRemoteHosts(addrs)
	if len(hosts) == 0 {
		hosts = []netip.Addr{netip.MustParseAddr("127.0.0.1")}
	}
	urls := make([]string, 0, len(hosts))
	seen := map[string]struct{}{}
	for _, addr := range hosts {
		if !addr.IsValid() || addr.IsUnspecified() {
			continue
		}
		raw := "ws://" + net.JoinHostPort(addr.String(), port)
		if _, ok := seen[raw]; ok {
			continue
		}
		seen[raw] = struct{}{}
		urls = append(urls, raw)
	}
	return urls
}

func localInterfaceAddrs() []netip.Addr {
	interfaces, err := net.Interfaces()
	if err != nil {
		return nil
	}
	var out []netip.Addr
	for _, iface := range interfaces {
		if iface.Flags&net.FlagUp == 0 {
			continue
		}
		addrs, err := iface.Addrs()
		if err != nil {
			continue
		}
		for _, raw := range addrs {
			prefix, err := netip.ParsePrefix(raw.String())
			if err == nil {
				out = append(out, prefix.Addr())
				continue
			}
			addr, err := netip.ParseAddr(raw.String())
			if err == nil {
				out = append(out, addr)
			}
		}
	}
	return out
}

func sortedRemoteHosts(addrs []netip.Addr) []netip.Addr {
	out := make([]netip.Addr, 0, len(addrs))
	for _, addr := range addrs {
		if addr.IsValid() && !addr.IsUnspecified() {
			out = append(out, addr)
		}
	}
	sort.SliceStable(out, func(i, j int) bool {
		left, right := remoteAddrRank(out[i]), remoteAddrRank(out[j])
		if left != right {
			return left < right
		}
		return out[i].String() < out[j].String()
	})
	return out
}

func remoteAddrRank(addr netip.Addr) int {
	tailscale := netip.MustParsePrefix("100.64.0.0/10")
	switch {
	case tailscale.Contains(addr):
		return 0
	case addr.IsPrivate():
		return 1
	case addr.IsLoopback():
		return 3
	default:
		return 2
	}
}

func isWildcardHost(host string) bool {
	if host == "" {
		return true
	}
	addr, err := netip.ParseAddr(host)
	if err != nil {
		return false
	}
	return addr.IsUnspecified()
}
