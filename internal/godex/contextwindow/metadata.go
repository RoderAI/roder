package contextwindow

const (
	DefaultOpenAIContextWindow = 272000
	DefaultOpenAICompactLimit  = 217600
)

type ModelWindow struct {
	Model                 string
	ContextWindow         int
	MaxContextWindow      int
	AutoCompactTokenLimit int
	SupportsCompaction    bool
}

func ForModel(modelID string) ModelWindow {
	if modelID == "" {
		modelID = "gpt-5.5"
	}
	if window, ok := modelWindows[modelID]; ok {
		window.Model = modelID
		return window
	}
	return ModelWindow{
		Model:                 modelID,
		ContextWindow:         DefaultOpenAIContextWindow,
		MaxContextWindow:      DefaultOpenAIContextWindow,
		AutoCompactTokenLimit: DefaultOpenAICompactLimit,
		SupportsCompaction:    false,
	}
}

var modelWindows = map[string]ModelWindow{
	"gpt-5.5": {
		ContextWindow:         1050000,
		MaxContextWindow:      1050000,
		AutoCompactTokenLimit: 800000,
		SupportsCompaction:    true,
	},
	"gpt-5.4": openAIWindow(DefaultOpenAIContextWindow),
	"gpt-5.4-mini": {
		ContextWindow:         DefaultOpenAIContextWindow,
		MaxContextWindow:      DefaultOpenAIContextWindow,
		AutoCompactTokenLimit: DefaultOpenAICompactLimit,
		SupportsCompaction:    true,
	},
	"gpt-5.3-codex": openAIWindow(DefaultOpenAIContextWindow),
	"gpt-5.2":       openAIWindow(DefaultOpenAIContextWindow),
	"codex-auto-review": {
		ContextWindow:         DefaultOpenAIContextWindow,
		MaxContextWindow:      DefaultOpenAIContextWindow,
		AutoCompactTokenLimit: DefaultOpenAICompactLimit,
		SupportsCompaction:    false,
	},
}

func openAIWindow(contextWindow int) ModelWindow {
	return ModelWindow{
		ContextWindow:         contextWindow,
		MaxContextWindow:      contextWindow,
		AutoCompactTokenLimit: int(float64(contextWindow) * 0.8),
		SupportsCompaction:    true,
	}
}
