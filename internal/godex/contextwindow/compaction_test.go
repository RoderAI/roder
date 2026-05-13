package contextwindow

import "testing"

func TestCompactionOptionsForGPT55(t *testing.T) {
	options := OptionsForModel("gpt-5.5", false, 0)
	if !options.Enabled {
		t.Fatal("compaction should be enabled")
	}
	if options.CompactThreshold != 800000 {
		t.Fatalf("threshold = %d", options.CompactThreshold)
	}
	if options.ContextWindow != 1050000 {
		t.Fatalf("context window = %d", options.ContextWindow)
	}
}

func TestCompactionOptionsCanBeDisabledAndClampsThreshold(t *testing.T) {
	options := OptionsForModel("gpt-5.5", true, 2000000)
	if options.Enabled {
		t.Fatal("compaction should be disabled")
	}
	if options.CompactThreshold != 1050000 {
		t.Fatalf("threshold should clamp to context window, got %d", options.CompactThreshold)
	}
}

func TestCompactionOptionsOmittedForUnknownModel(t *testing.T) {
	options := OptionsForModel("gpt-future", false, 0)
	if options.Enabled {
		t.Fatal("unknown model should not enable server-side compaction")
	}
}
