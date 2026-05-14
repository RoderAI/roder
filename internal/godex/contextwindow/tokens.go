package contextwindow

import (
	"encoding/json"
	"strings"
	"sync"
	"unicode/utf8"

	tiktoken "github.com/pkoukk/tiktoken-go"
)

const (
	messageTokenOverhead    = 6
	toolTokenOverhead       = 12
	rawItemTokenOverhead    = 24
	imageTokenOverhead      = 85
	requestTokenOverhead    = 16
	itemTokenOverhead       = 8
	toolSpecTokenOverhead   = 20
	charactersPerTokenGuess = 2
	requestSafetyMultiplier = 1.5
)

type Message struct {
	Role          string
	Content       string
	Phase         string
	Images        []Image
	ToolCallID    string
	ToolName      string
	ToolArguments string
	RawJSON       json.RawMessage
}

type Image struct {
	URL    string
	Detail string
}

type Item struct {
	Kind       string
	Role       string
	Phase      string
	ToolName   string
	ToolCallID string
	Text       string
	Images     []Image
	RawJSON    json.RawMessage
}

type ToolSpec struct {
	Name        string
	Description string
	Schema      map[string]any
}

type Request struct {
	Model          string
	Instructions   string
	ResponseFormat string
	Messages       []Message
	InputItems     []Item
	Tools          []ToolSpec
}

type TokenEstimate struct {
	Tokens        int
	ContextWindow int
	Percent       float64
}

var encodings sync.Map

func EstimateRequest(req Request, window ModelWindow) TokenEstimate {
	tokens := requestTokenOverhead
	encoder := encoderForModel(req.Model)
	tokens += estimateTextWithEncoder(req.Instructions, encoder)
	tokens += estimateTextWithEncoder(req.ResponseFormat, encoder)
	if len(req.InputItems) > 0 {
		for _, item := range req.InputItems {
			tokens += EstimateItem(item, encoder)
		}
	} else {
		for _, msg := range req.Messages {
			tokens += EstimateMessageWithModel(msg, encoder)
		}
	}
	for _, tool := range req.Tools {
		tokens += EstimateToolSpec(tool, encoder)
	}
	tokens = int(float64(tokens)*requestSafetyMultiplier + 0.5)
	return tokenEstimate(tokens, window)
}

func EstimateMessages(messages []Message, window ModelWindow) TokenEstimate {
	tokens := 0
	encoder := encoderForModel("")
	for _, msg := range messages {
		tokens += EstimateMessageWithModel(msg, encoder)
	}
	return tokenEstimate(tokens, window)
}

func EstimateMessage(msg Message) int {
	return EstimateMessageWithModel(msg, encoderForModel(""))
}

func EstimateMessageWithModel(msg Message, encoder *tiktoken.Tiktoken) int {
	if len(msg.RawJSON) > 0 {
		return rawItemTokenOverhead + estimateTextWithEncoder(string(msg.RawJSON), encoder)
	}
	overhead := messageTokenOverhead
	if msg.ToolCallID != "" || msg.ToolName != "" || msg.ToolArguments != "" {
		overhead += toolTokenOverhead
	}
	return overhead +
		estimateTextWithEncoder(msg.Role, encoder) +
		estimateTextWithEncoder(msg.Phase, encoder) +
		estimateTextWithEncoder(msg.Content, encoder) +
		estimateImages(msg.Images, encoder) +
		estimateTextWithEncoder(msg.ToolName, encoder) +
		estimateTextWithEncoder(msg.ToolCallID, encoder) +
		estimateTextWithEncoder(msg.ToolArguments, encoder)
}

func EstimateItem(item Item, encoder *tiktoken.Tiktoken) int {
	if len(item.RawJSON) > 0 {
		return rawItemTokenOverhead + estimateTextWithEncoder(string(item.RawJSON), encoder)
	}
	return itemTokenOverhead +
		estimateTextWithEncoder(item.Kind, encoder) +
		estimateTextWithEncoder(item.Role, encoder) +
		estimateTextWithEncoder(item.Phase, encoder) +
		estimateTextWithEncoder(item.ToolName, encoder) +
		estimateTextWithEncoder(item.ToolCallID, encoder) +
		estimateTextWithEncoder(item.Text, encoder) +
		estimateImages(item.Images, encoder)
}

func EstimateToolSpec(tool ToolSpec, encoder *tiktoken.Tiktoken) int {
	tokens := toolSpecTokenOverhead +
		estimateTextWithEncoder(tool.Name, encoder) +
		estimateTextWithEncoder(tool.Description, encoder)
	if tool.Schema != nil {
		data, err := json.Marshal(tool.Schema)
		if err == nil {
			tokens += estimateTextWithEncoder(string(data), encoder)
		}
	}
	return tokens
}

func EstimateText(text string) int {
	return estimateTextWithEncoder(text, encoderForModel(""))
}

func tokenEstimate(tokens int, window ModelWindow) TokenEstimate {
	estimate := TokenEstimate{Tokens: tokens, ContextWindow: window.ContextWindow}
	if window.ContextWindow > 0 {
		estimate.Percent = float64(tokens) / float64(window.ContextWindow) * 100
	}
	return estimate
}

func estimateImages(images []Image, encoder *tiktoken.Tiktoken) int {
	tokens := 0
	for _, image := range images {
		tokens += imageTokenOverhead
		tokens += estimateTextWithEncoder(image.URL, encoder)
		tokens += estimateTextWithEncoder(image.Detail, encoder)
	}
	return tokens
}

func estimateTextWithEncoder(text string, encoder *tiktoken.Tiktoken) int {
	if text == "" {
		return 0
	}
	if encoder != nil {
		return encodeTokenCount(text, encoder)
	}
	tokens := len([]rune(text)) / charactersPerTokenGuess
	if tokens == 0 {
		return 1
	}
	return tokens
}

func encodeTokenCount(text string, encoder *tiktoken.Tiktoken) int {
	const chunkBytes = 16 * 1024
	const exactLimitBytes = 256 * 1024
	if len(text) <= chunkBytes {
		return len(encoder.EncodeOrdinary(text))
	}
	if len(text) > exactLimitBytes {
		return sampledTokenCount(text, encoder, chunkBytes)
	}
	tokens := 0
	for start := 0; start < len(text); {
		end := start + chunkBytes
		if end > len(text) {
			end = len(text)
		} else if end < len(text) {
			for end > start && !utf8.RuneStart(text[end]) {
				end--
			}
			if end == start {
				end = start + chunkBytes
			}
		}
		tokens += len(encoder.EncodeOrdinary(text[start:end]))
		start = end
	}
	return tokens
}

func sampledTokenCount(text string, encoder *tiktoken.Tiktoken, sampleBytes int) int {
	type span struct {
		start int
		end   int
	}
	mid := len(text) / 2
	spans := []span{
		{start: 0, end: sampleBytes},
		{start: max(0, mid-sampleBytes/2), end: min(len(text), mid+sampleBytes/2)},
		{start: max(0, len(text)-sampleBytes), end: len(text)},
	}
	sampleRunes := 0
	sampleTokens := 0
	for _, s := range spans {
		start, end := utf8Span(text, s.start, s.end)
		if end <= start {
			continue
		}
		part := text[start:end]
		sampleRunes += utf8.RuneCountInString(part)
		sampleTokens += len(encoder.EncodeOrdinary(part))
	}
	if sampleRunes == 0 {
		return 0
	}
	totalRunes := utf8.RuneCountInString(text)
	estimate := float64(sampleTokens) / float64(sampleRunes) * float64(totalRunes)
	return int(estimate*1.08 + 0.5)
}

func utf8Span(text string, start int, end int) (int, int) {
	start = clampIndex(start, len(text))
	end = clampIndex(end, len(text))
	for start < len(text) && !utf8.RuneStart(text[start]) {
		start++
	}
	for end > start && end < len(text) && !utf8.RuneStart(text[end]) {
		end--
	}
	return start, end
}

func clampIndex(value int, limit int) int {
	if value < 0 {
		return 0
	}
	if value > limit {
		return limit
	}
	return value
}

func encoderForModel(model string) *tiktoken.Tiktoken {
	key := strings.TrimSpace(model)
	if key == "" {
		key = "o200k_base"
	}
	if value, ok := encodings.Load(key); ok {
		if encoder, ok := value.(*tiktoken.Tiktoken); ok {
			return encoder
		}
		return nil
	}
	encoder, err := tiktoken.EncodingForModel(key)
	if err != nil {
		encoder, err = tiktoken.GetEncoding(tiktoken.MODEL_O200K_BASE)
	}
	if err != nil {
		return nil
	}
	actual, _ := encodings.LoadOrStore(key, encoder)
	if cached, ok := actual.(*tiktoken.Tiktoken); ok {
		return cached
	}
	return nil
}
