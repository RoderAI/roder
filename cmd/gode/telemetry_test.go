package main

import "testing"

func TestParseConfigTelemetryFlags(t *testing.T) {
	cfg, err := parseConfig([]string{
		"--telemetry",
		"--telemetry-endpoint", "127.0.0.1:4317",
	})
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	if !cfg.Telemetry {
		t.Fatal("telemetry = false")
	}
	if cfg.TelemetryEndpoint != "127.0.0.1:4317" {
		t.Fatalf("telemetry endpoint = %q", cfg.TelemetryEndpoint)
	}
}
