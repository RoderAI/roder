package builtin

import (
	"context"
	"fmt"
	"strings"
	"sync"

	"github.com/pandelisz/gode/internal/godex/tools"
)

type todoItem struct {
	Content string
	Status  string
}

func RegisterTodo(reg *tools.Registry) {
	var mu sync.Mutex
	var items []todoItem
	reg.Register(tools.Tool{
		Name:        "todo_update",
		Description: "Replace the current todo list with ordered items.",
		ReadOnly:    false,
		Schema:      objectSchema("items"),
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			mu.Lock()
			defer mu.Unlock()
			items = items[:0]
			for _, raw := range arrayInput(call.Input, "items") {
				itemMap, ok := raw.(map[string]any)
				if !ok {
					continue
				}
				content := fmt.Sprint(itemMap["content"])
				status := fmt.Sprint(itemMap["status"])
				if content == "" || content == "<nil>" {
					continue
				}
				if status == "" || status == "<nil>" {
					status = "pending"
				}
				items = append(items, todoItem{Content: content, Status: status})
			}
			var lines []string
			for _, item := range items {
				lines = append(lines, fmt.Sprintf("[%s] %s", item.Status, item.Content))
			}
			return tools.Result{Text: strings.Join(lines, "\n"), Data: items}, nil
		},
	})
}
