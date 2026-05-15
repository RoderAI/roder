package contextwindow

import "testing"

func TestGPT55ContextWindow(t *testing.T) {
	window := ForModel("gpt-5.5")
	if window.ContextWindow != 1050000 {
		t.Fatalf("context window = %d", window.ContextWindow)
	}
	if window.MaxContextWindow != 1050000 {
		t.Fatalf("max context window = %d", window.MaxContextWindow)
	}
	if window.AutoCompactTokenLimit != 600000 {
		t.Fatalf("auto compact limit = %d", window.AutoCompactTokenLimit)
	}
	if !window.SupportsCompaction {
		t.Fatal("gpt-5.5 should support compaction")
	}
}

func TestGeminiContextWindow(t *testing.T) {
	for _, id := range []string{"gemini-3.1-pro-preview", "gemini-3.1-pro-preview-customtools", "gemini-3-flash-preview", "gemini-3.1-flash-lite-preview"} {
		window := ForModel(id)
		if window.ContextWindow != 1048576 || window.MaxContextWindow != 1048576 {
			t.Fatalf("%s context = %d/%d", id, window.ContextWindow, window.MaxContextWindow)
		}
		if window.AutoCompactTokenLimit != 838860 {
			t.Fatalf("%s compact limit = %d", id, window.AutoCompactTokenLimit)
		}
		if window.SupportsCompaction {
			t.Fatalf("%s should not claim provider-side compaction", id)
		}
	}
}

func TestFallbackOpenAIModelDoesNotClaimOneMillionContext(t *testing.T) {
	window := ForModel("gpt-future")
	if window.ContextWindow != DefaultOpenAIContextWindow {
		t.Fatalf("context window = %d", window.ContextWindow)
	}
	if window.AutoCompactTokenLimit != DefaultOpenAICompactLimit {
		t.Fatalf("compact limit = %d", window.AutoCompactTokenLimit)
	}
	if window.SupportsCompaction {
		t.Fatal("fallback model should not claim compaction support")
	}
}
