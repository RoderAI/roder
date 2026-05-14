package appserver

import (
	"strings"
	"testing"
)

func TestRenderTerminalQR(t *testing.T) {
	got, err := RenderTerminalQR("gode://connect?payload=test")
	if err != nil {
		t.Fatalf("render qr: %v", err)
	}
	if strings.TrimSpace(got) == "" {
		t.Fatal("qr output was empty")
	}
	if !strings.Contains(got, "█") && !strings.Contains(got, "▀") {
		t.Fatalf("qr output does not look terminal-rendered: %q", got)
	}
}
