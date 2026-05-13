package builtin

import (
	"bufio"
	"context"
	"encoding/json"
	"os"
	"strings"
	"time"

	"github.com/pandelisz/gode/internal/godex/tools"
)

type memoryEntry struct {
	Time time.Time `json:"time"`
	Note string    `json:"note"`
}

func RegisterMemory(reg *tools.Registry, path string) {
	reg.Register(tools.Tool{
		Name:        "memory_add",
		Description: "Append a durable note to the local memory journal.",
		ReadOnly:    false,
		Schema:      objectSchema("note"),
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			entry := memoryEntry{Time: time.Now().UTC(), Note: stringInput(call.Input, "note")}
			file, err := os.OpenFile(path, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0o600)
			if err != nil {
				return tools.Result{}, err
			}
			defer file.Close()
			data, err := json.Marshal(entry)
			if err != nil {
				return tools.Result{}, err
			}
			if _, err := file.Write(append(data, '\n')); err != nil {
				return tools.Result{}, err
			}
			return tools.Result{Text: "memory added"}, nil
		},
	})
	reg.Register(tools.Tool{
		Name:        "memory_list",
		Description: "List local memory notes.",
		ReadOnly:    true,
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			file, err := os.Open(path)
			if os.IsNotExist(err) {
				return tools.Result{Text: ""}, nil
			}
			if err != nil {
				return tools.Result{}, err
			}
			defer file.Close()
			var lines []string
			scanner := bufio.NewScanner(file)
			for scanner.Scan() {
				var entry memoryEntry
				if err := json.Unmarshal(scanner.Bytes(), &entry); err != nil {
					continue
				}
				lines = append(lines, entry.Note)
			}
			return tools.Result{Text: strings.Join(lines, "\n")}, scanner.Err()
		},
	})
}
