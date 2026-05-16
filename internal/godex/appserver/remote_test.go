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
	if parsed.Scheme != "gode" || parsed.Host != "192.168.1.12:43210" {
		t.Fatalf("deep link target = %s://%s", parsed.Scheme, parsed.Host)
	}
	query := parsed.Query()
	if query.Get("auth") != "secret-token" {
		t.Fatalf("decoded query mismatch: %#v", query)
	}
	if strings.Contains(link, "ws://") {
		t.Fatalf("deep link should imply websocket scheme, got %s", link)
	}
	if len(link) >= 80 {
		t.Fatalf("deep link should stay compact, length = %d: %s", len(link), link)
	}
}

func TestRemoteConnectURLsSortsUsableHosts(t *testing.T) {
	addrs := []netip.Addr{
		netip.MustParseAddr("127.0.0.1"),
		netip.MustParseAddr("192.168.1.20"),
		netip.MustParseAddr("100.99.1.2"),
	}
	got := RemoteConnectURLs("0.0.0.0:43210", addrs)
	want := []string{"ws://192.168.1.20:43210", "ws://100.99.1.2:43210", "ws://127.0.0.1:43210"}
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
