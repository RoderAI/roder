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
