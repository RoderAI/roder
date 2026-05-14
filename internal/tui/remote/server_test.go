package remote

import (
	"context"
	"path/filepath"
	"testing"

	"github.com/pandelisz/gode/internal/godex"
)

func TestControllerStartStop(t *testing.T) {
	ctx := context.Background()
	app, err := godex.New(ctx, godex.Config{
		Workspace:   filepath.Join(t.TempDir(), "workspace"),
		DataDir:     t.TempDir(),
		Provider:    "mock",
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("new app: %v", err)
	}
	defer app.Close(ctx)

	controller := NewController(app)
	state, err := controller.Start(ctx)
	if err != nil {
		t.Fatalf("start remote: %v", err)
	}
	if !state.Running || len(state.URLs) == 0 || state.TokenPreview == "" || state.QR == "" {
		t.Fatalf("started state incomplete: %#v", state)
	}
	if state.AuthHeaderHint == "" || state.SubprotocolHint == "" {
		t.Fatalf("auth hints missing: %#v", state)
	}
	stopped, err := controller.Stop(ctx)
	if err != nil {
		t.Fatalf("stop remote: %v", err)
	}
	if stopped.Running || stopped.TokenPreview != "" {
		t.Fatalf("stopped state kept remote data: %#v", stopped)
	}
}
