package godex

import "testing"

func TestDefaultConfigDisablesTelemetryWithJaegerEndpoint(t *testing.T) {
	cfg := DefaultConfig()
	if cfg.Telemetry {
		t.Fatal("telemetry should be disabled by default")
	}
	if cfg.TelemetryEndpoint != "localhost:4317" {
		t.Fatalf("telemetry endpoint = %q, want localhost:4317", cfg.TelemetryEndpoint)
	}
}
