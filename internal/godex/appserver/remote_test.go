package appserver

import (
	"net/netip"
	"net/url"
	"strings"
	"testing"
)

func TestRemotePairingPayloadAndDeepLink(t *testing.T) {
	payload := BuildRemotePairingPayload("Gode Remote", "ws://192.168.1.12:43210", "secret-token", "/repo")
	if strings.Contains(payload.URL, "secret-token") {
		t.Fatal("websocket URL contains token")
	}
	if payload.Headers["Authorization"] != "Bearer secret-token" {
		t.Fatalf("auth header = %q", payload.Headers["Authorization"])
	}
	link, err := RemoteDeepLink(payload)
	if err != nil {
		t.Fatalf("deep link: %v", err)
	}
	parsed, err := url.Parse(link)
	if err != nil {
		t.Fatalf("parse deep link: %v", err)
	}
	if parsed.Scheme != "gode" || parsed.Host != "connect" {
		t.Fatalf("deep link target = %s://%s", parsed.Scheme, parsed.Host)
	}
	if parsed.Query().Get("auth") != "" {
		t.Fatalf("deep link must not use raw auth query: %s", link)
	}
	decoded, err := DecodeRemoteDeepLink(link)
	if err != nil {
		t.Fatalf("decode deep link: %v", err)
	}
	if decoded.URL != payload.URL || decoded.Headers["Authorization"] != "Bearer secret-token" || decoded.Workspace != "/repo" {
		t.Fatalf("decoded payload = %#v", decoded)
	}
	if strings.Contains(link, "ws://") || strings.Contains(link, "secret-token") {
		t.Fatalf("deep link should encode payload, got %s", link)
	}
	if parsed.Query().Get("payload") == "" {
		t.Fatalf("deep link missing encoded payload: %s", link)
	}
}

func TestRemoteConnectURLsSortsUsableHosts(t *testing.T) {
	addrs := []netip.Addr{
		netip.MustParseAddr("127.0.0.1"),
		netip.MustParseAddr("192.168.1.20"),
		netip.MustParseAddr("100.99.1.2"),
	}
	got := RemoteConnectURLs("0.0.0.0:43210", addrs)
	want := []string{"ws://100.99.1.2:43210", "ws://192.168.1.20:43210", "ws://127.0.0.1:43210"}
	if strings.Join(got, "\n") != strings.Join(want, "\n") {
		t.Fatalf("urls = %#v, want %#v", got, want)
	}
}

func TestRemoteConnectURLsKeepExplicitHost(t *testing.T) {
	got := RemoteConnectURLs("127.0.0.1:43210", []netip.Addr{netip.MustParseAddr("100.99.1.2")})
	if len(got) != 1 || got[0] != "ws://127.0.0.1:43210" {
		t.Fatalf("urls = %#v", got)
	}
}
