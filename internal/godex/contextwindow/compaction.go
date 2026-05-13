package contextwindow

type CompactionOptions struct {
	Enabled          bool
	Model            string
	ContextWindow    int
	CompactThreshold int
}

func OptionsForModel(modelID string, disabled bool, thresholdOverride int) CompactionOptions {
	window := ForModel(modelID)
	if !window.SupportsCompaction || disabled {
		return CompactionOptions{
			Model:            window.Model,
			ContextWindow:    window.ContextWindow,
			CompactThreshold: clampThreshold(thresholdOverride, window),
		}
	}
	return CompactionOptions{
		Enabled:          true,
		Model:            window.Model,
		ContextWindow:    window.ContextWindow,
		CompactThreshold: clampThreshold(thresholdOverride, window),
	}
}

func clampThreshold(threshold int, window ModelWindow) int {
	if threshold <= 0 {
		threshold = window.AutoCompactTokenLimit
	}
	if threshold <= 0 {
		threshold = int(float64(window.ContextWindow) * 0.8)
	}
	if window.ContextWindow > 0 && threshold > window.ContextWindow {
		return window.ContextWindow
	}
	return threshold
}
