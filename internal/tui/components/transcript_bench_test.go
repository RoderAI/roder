package components

import (
	"fmt"
	"strings"
	"testing"

	zone "github.com/lrstanley/bubblezone/v2"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func BenchmarkTranscriptScrollWindow(b *testing.B) {
	messages := benchmarkMessages(500, 900)
	zones := zone.New()

	for _, scrollOffset := range []int{0, 240, 1200} {
		b.Run(fmt.Sprintf("offset_%d", scrollOffset), func(b *testing.B) {
			b.ReportAllocs()
			for i := 0; i < b.N; i++ {
				_ = Transcript(120, 36, messages, scrollOffset, "", zones)
			}
		})
	}
}

func BenchmarkTranscriptCachedScrollWindow(b *testing.B) {
	messages := benchmarkMessages(500, 900)
	zones := zone.New()
	cache := NewTranscriptCache()

	for _, scrollOffset := range []int{0, 240, 1200} {
		b.Run(fmt.Sprintf("offset_%d", scrollOffset), func(b *testing.B) {
			b.ReportAllocs()
			for i := 0; i < b.N; i++ {
				_ = TranscriptWithCache(120, 36, messages, scrollOffset, "", zones, &cache)
			}
		})
	}
}

func benchmarkMessages(count int, bodyBytes int) []viewmodel.Message {
	messages := make([]viewmodel.Message, count)
	seed := strings.Repeat("scrollable transcript output with wrapped words and tool result fragments ", bodyBytes/70+1)
	for i := range messages {
		role := viewmodel.RoleAssistant
		title := ""
		if i%5 == 0 {
			role = viewmodel.RoleTool
			title = "shell"
		}
		messages[i] = viewmodel.Message{
			ID:    fmt.Sprintf("m%d", i),
			Role:  role,
			Title: title,
			Body:  seed[:bodyBytes],
		}
	}
	return messages
}
