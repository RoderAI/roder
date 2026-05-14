package memory

import (
	"context"
	"strconv"
	"strings"
)

const (
	MaxRecallPreviewChars = 240
	MaxRecallSnippetBytes = 2000
)

type RecallResult struct {
	Query     string
	Model     string
	Entries   []Entry
	MemoryIDs []string
	Text      string
}

func (s *Service) Recall(ctx context.Context, query string) (RecallResult, error) {
	query = normalizeContent(query)
	if s == nil || !s.cfg.Enabled || !s.cfg.AutoRecall || query == "" {
		return RecallResult{Query: query}, nil
	}
	entries, err := s.Query(ctx, query, s.cfg.RecallLimit)
	if err != nil {
		return RecallResult{Query: query, Model: s.cfg.EmbeddingModel}, err
	}
	ids := make([]string, 0, len(entries))
	for _, entry := range entries {
		ids = append(ids, entry.ID)
	}
	return RecallResult{
		Query:     query,
		Model:     s.cfg.EmbeddingModel,
		Entries:   entries,
		MemoryIDs: ids,
		Text:      FormatRecallSnippets(entries),
	}, nil
}

func FormatRecallSnippets(entries []Entry) string {
	if len(entries) == 0 {
		return ""
	}
	var b strings.Builder
	b.WriteString("Relevant local memories for this workspace:\n\n")
	for i, entry := range entries {
		id := strings.TrimSpace(entry.ID)
		preview := truncateRunes(entry.Content, MaxRecallPreviewChars)
		if id == "" || preview == "" {
			continue
		}
		next := strconv.Itoa(i+1) + ". [" + id + "] " + preview
		if b.Len()+len(next)+2+len("\nUse read_memory with the memory ID if full detail is needed.") > MaxRecallSnippetBytes {
			break
		}
		b.WriteString(next)
		b.WriteByte('\n')
	}
	b.WriteString("\nUse read_memory with the memory ID if full detail is needed.")
	text := b.String()
	if len(text) > MaxRecallSnippetBytes {
		return text[:MaxRecallSnippetBytes]
	}
	return text
}

func truncateRunes(text string, limit int) string {
	text = strings.TrimSpace(text)
	runes := []rune(text)
	if len(runes) <= limit {
		return text
	}
	if limit <= 3 {
		return string(runes[:limit])
	}
	return string(runes[:limit-3]) + "..."
}
