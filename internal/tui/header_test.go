package tui

import (
	"context"
	"path/filepath"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex"
)

func TestHeaderShowsReasoningWithModel(t *testing.T) {
	app, err := godex.New(context.Background(), godex.Config{
		DataDir:     t.TempDir(),
		Workspace:   filepath.Join(t.TempDir(), "workspace"),
		Provider:    godex.ProviderOpenAI,
		Model:       godex.DefaultModelID,
		Reasoning:   godex.ReasoningHigh,
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	defer app.Close(context.Background())

	model := New(app)
	view := model.View().Content
	if !strings.Contains(view, godex.DefaultModelID+" "+godex.ReasoningHigh) {
		t.Fatalf("header should show model and reasoning:\n%s", view)
	}
}

func TestHeaderShowsSessionTitleAndRunningState(t *testing.T) {
	model := New(nil)
	model.width = 100
	model.height = 24
	model.currentSession = "Implement parser"
	model.running = true

	view := model.View().Content
	for _, want := range []string{"Implement parser", "running"} {
		if !strings.Contains(view, want) {
			t.Fatalf("header missing %q:\n%s", want, view)
		}
	}
}
