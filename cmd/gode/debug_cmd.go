package main

import (
	"context"
	"fmt"

	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/contextwindow"
	messagestore "github.com/pandelisz/gode/internal/godex/message"
)

func runDebug(args []string) error {
	if len(args) == 0 || args[0] != "context" {
		return fmt.Errorf("usage: gode debug context --session <id>")
	}
	flags := newFlagSet("gode debug context")
	cfg := godex.DefaultConfig()
	sessionID := ""
	flags.StringVar(&sessionID, "session", sessionID, "session id to inspect")
	bindConfigFlags(flags, &cfg)
	if err := flags.Parse(args[1:]); err != nil {
		return err
	}
	if sessionID == "" {
		return fmt.Errorf("usage: gode debug context --session <id>")
	}
	loaded, err := loadConfigFromFlags(cfg, flags)
	if err != nil {
		return err
	}
	cfg = loaded.Config
	store := messagestore.Open(cfg.DataDir)
	messages, err := store.ListBySession(context.Background(), sessionID)
	if err != nil {
		return err
	}
	window := contextwindow.ForModel(cfg.Model)
	estimate := contextwindow.EstimateMessages(contextMessagesFromStored(messages), window)
	compaction := contextwindow.OptionsForModel(cfg.Model, cfg.DisableAutoCompaction, cfg.AutoCompactTokenLimit)

	fmt.Printf("session_id\t%s\n", sessionID)
	fmt.Printf("model\t%s\n", cfg.Model)
	fmt.Printf("estimated_tokens\t%d\n", estimate.Tokens)
	fmt.Printf("context_window\t%d\n", estimate.ContextWindow)
	fmt.Printf("context_used_percent\t%.2f\n", estimate.Percent)
	fmt.Printf("auto_compaction_enabled\t%t\n", compaction.Enabled)
	fmt.Printf("compact_threshold\t%d\n", compaction.CompactThreshold)
	fmt.Printf("supports_compaction\t%t\n", window.SupportsCompaction)
	fmt.Printf("message_count\t%d\n", len(messages))
	return nil
}

func contextMessagesFromStored(messages []messagestore.Message) []contextwindow.Message {
	out := make([]contextwindow.Message, 0, len(messages))
	for _, msg := range messages {
		out = append(out, contextwindow.Message{
			Role:       msg.Role,
			Content:    msg.Text,
			ToolName:   msg.ToolName,
			ToolCallID: msg.ToolCallID,
		})
	}
	return out
}
