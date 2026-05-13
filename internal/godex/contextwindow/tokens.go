package contextwindow

import "encoding/json"

const (
	messageTokenOverhead    = 6
	toolTokenOverhead       = 12
	rawItemTokenOverhead    = 24
	charactersPerTokenGuess = 3
)

type Message struct {
	Role          string
	Content       string
	ToolCallID    string
	ToolName      string
	ToolArguments string
	RawJSON       json.RawMessage
}

type TokenEstimate struct {
	Tokens        int
	ContextWindow int
	Percent       float64
}

func EstimateMessages(messages []Message, window ModelWindow) TokenEstimate {
	tokens := 0
	for _, msg := range messages {
		tokens += EstimateMessage(msg)
	}
	estimate := TokenEstimate{Tokens: tokens, ContextWindow: window.ContextWindow}
	if window.ContextWindow > 0 {
		estimate.Percent = float64(tokens) / float64(window.ContextWindow) * 100
	}
	return estimate
}

func EstimateMessage(msg Message) int {
	if len(msg.RawJSON) > 0 {
		return rawItemTokenOverhead + EstimateText(string(msg.RawJSON))
	}
	overhead := messageTokenOverhead
	if msg.ToolCallID != "" || msg.ToolName != "" || msg.ToolArguments != "" {
		overhead += toolTokenOverhead
	}
	return overhead + EstimateText(msg.Role) + EstimateText(msg.Content) + EstimateText(msg.ToolName) + EstimateText(msg.ToolCallID) + EstimateText(msg.ToolArguments)
}

func EstimateText(text string) int {
	if text == "" {
		return 0
	}
	tokens := len([]rune(text)) / charactersPerTokenGuess
	if tokens == 0 {
		return 1
	}
	return tokens
}
