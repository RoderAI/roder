package memory

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"

	"github.com/pandelisz/gode/internal/godex/tools"
)

type ToolEntry struct {
	ID        string  `json:"id"`
	Content   string  `json:"content"`
	UpdatedAt string  `json:"updated_at"`
	Score     float64 `json:"score,omitempty"`
}

func RegisterTools(reg *tools.Registry, service *Service) {
	reg.Register(tools.Tool{
		Name:        "memory_save",
		Description: "Save a durable semantic memory for the current workspace.",
		Schema:      objectSchema("content"),
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			entry, err := service.Save(ctx, stringInput(call.Input, "content"), "tool")
			if err != nil {
				return tools.Result{}, err
			}
			row := toolEntry(entry)
			return tools.Result{Text: fmt.Sprintf("saved memory %s", row.ID), Data: row}, nil
		},
	})
	reg.Register(tools.Tool{
		Name:        "memory_update",
		Description: "Update a durable semantic memory by ID.",
		Schema:      objectSchema("id", "content"),
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			entry, err := service.Update(ctx, stringInput(call.Input, "id"), stringInput(call.Input, "content"))
			if err != nil {
				return tools.Result{}, err
			}
			row := toolEntry(entry)
			return tools.Result{Text: fmt.Sprintf("updated memory %s", row.ID), Data: row}, nil
		},
	})
	reg.Register(tools.Tool{
		Name:        "memory_delete",
		Description: "Delete a durable semantic memory by ID.",
		Schema:      objectSchema("id"),
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			id := stringInput(call.Input, "id")
			if err := service.Delete(ctx, id); err != nil {
				return tools.Result{}, err
			}
			return tools.Result{Text: fmt.Sprintf("deleted memory %s", strings.TrimSpace(id))}, nil
		},
	})
	reg.Register(tools.Tool{
		Name:        "memory_find",
		Description: "Search durable semantic memories for the current workspace.",
		Schema:      objectSchema("query"),
		ReadOnly:    true,
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			entries, err := service.Query(ctx, stringInput(call.Input, "query"), intInputDefault(call.Input, "limit", service.cfg.RecallLimit))
			if err != nil {
				return tools.Result{}, err
			}
			rows := toolEntries(entries)
			text, err := jsonText(rows)
			if err != nil {
				return tools.Result{}, err
			}
			return tools.Result{Text: text, Data: rows}, nil
		},
	})
	reg.Register(tools.Tool{
		Name:        "memory_read",
		Description: "Read a durable semantic memory by ID.",
		Schema:      objectSchema("id"),
		ReadOnly:    true,
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			entry, err := service.Read(ctx, stringInput(call.Input, "id"))
			if err != nil {
				return tools.Result{}, err
			}
			return tools.Result{Text: entry.Content, Data: toolEntry(entry)}, nil
		},
	})
}

func toolEntries(entries []Entry) []ToolEntry {
	rows := make([]ToolEntry, 0, len(entries))
	for _, entry := range entries {
		rows = append(rows, toolEntry(entry))
	}
	return rows
}

func toolEntry(entry Entry) ToolEntry {
	return ToolEntry{
		ID:        entry.ID,
		Content:   entry.Content,
		UpdatedAt: entry.UpdatedAt.Format("2006-01-02T15:04:05.000000000Z07:00"),
		Score:     entry.Score,
	}
}

func jsonText(value any) (string, error) {
	data, err := json.MarshalIndent(value, "", "  ")
	if err != nil {
		return "", err
	}
	return string(data), nil
}

func objectSchema(required ...string) map[string]any {
	properties := map[string]any{}
	for _, name := range required {
		properties[name] = map[string]any{"type": "string"}
	}
	return map[string]any{
		"type":       "object",
		"properties": properties,
		"required":   append([]string(nil), required...),
	}
}

func stringInput(input map[string]any, key string) string {
	if input == nil {
		return ""
	}
	value, ok := input[key]
	if !ok || value == nil {
		return ""
	}
	if text, ok := value.(string); ok {
		return text
	}
	return ""
}

func intInputDefault(input map[string]any, key string, fallback int) int {
	if input == nil {
		return fallback
	}
	switch value := input[key].(type) {
	case int:
		return value
	case int64:
		return int(value)
	case float64:
		return int(value)
	default:
		return fallback
	}
}
