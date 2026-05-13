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
