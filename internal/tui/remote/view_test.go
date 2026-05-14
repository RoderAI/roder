package remote

import (
	"strings"
	"testing"
)

func TestSecurityWarning(t *testing.T) {
	if got := SecurityWarning(State{Running: true, URLs: []string{"ws://127.0.0.1:1"}}); got != "" {
		t.Fatalf("loopback warning = %q", got)
	}
	got := SecurityWarning(State{Running: true, URLs: []string{"ws://192.168.1.2:1"}})
	if !strings.Contains(got, "without TLS") {
		t.Fatalf("warning = %q", got)
	}
}
