package main

import (
	"context"
	"fmt"
	"strings"

	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/memory"
)

var memoryCommandEmbedderFactory = defaultMemoryCommandEmbedder

func defaultMemoryCommandEmbedder(model string) memory.Embedder {
	return memory.NewOpenAIEmbedder(model)
}

func runMemory(ctx context.Context, args []string) error {
	if len(args) == 0 {
		return fmt.Errorf("usage: gode memory list|query|enable|disable")
	}
	switch args[0] {
	case "list":
		return runMemoryList(ctx, args[1:])
	case "query":
		return runMemoryQuery(ctx, args[1:])
	case "enable":
		return runMemorySetEnabled(args[1:], true)
	case "disable":
		return runMemorySetEnabled(args[1:], false)
	default:
		return fmt.Errorf("unknown memory command %q", args[0])
	}
}

func runMemoryList(ctx context.Context, args []string) error {
	cfg, limit, err := parseMemoryCommandConfig("gode memory list", args, true)
	if err != nil {
		return err
	}
	service, closeService, err := newMemoryCommandService(ctx, cfg)
	if err != nil {
		return err
	}
	defer closeService()
	entries, err := service.List(ctx, limit)
	if err != nil {
		return err
	}
	fmt.Println("id\tupdated_at\tpreview")
	for _, entry := range entries {
		fmt.Printf("%s\t%s\t%s\n", entry.ID, entry.UpdatedAt.Format(timeFormat), previewMemoryContent(entry.Content))
	}
	return nil
}

func runMemoryQuery(ctx context.Context, args []string) error {
	cfg, limit, err := parseMemoryCommandConfig("gode memory query", args, true)
	if err != nil {
		return err
	}
	query := strings.TrimSpace(strings.Join(memoryCommandArgs(args), " "))
	if query == "" {
		return fmt.Errorf("query text is required")
	}
	service, closeService, err := newMemoryCommandService(ctx, cfg)
	if err != nil {
		return err
	}
	defer closeService()
	entries, err := service.Query(ctx, query, limit)
	if err != nil {
		return err
	}
	fmt.Println("id\tscore\tupdated_at\tpreview")
	for _, entry := range entries {
		fmt.Printf("%s\t%.3f\t%s\t%s\n", entry.ID, entry.Score, entry.UpdatedAt.Format(timeFormat), previewMemoryContent(entry.Content))
	}
	return nil
}

func runMemorySetEnabled(args []string, enabled bool) error {
	flags := newFlagSet("gode memory")
	cfg := godex.DefaultConfig()
	bindConfigFlags(flags, &cfg)
	if err := flags.Parse(args); err != nil {
		return err
	}
	loaded, err := loadConfigFromFlags(cfg, flags)
	if err != nil {
		return err
	}
	cfg = loaded.Config
	settings, err := godex.LoadSettings(cfg.DataDir)
	if err != nil {
		return err
	}
	settings.Memories = memorySettingsFromConfig(cfg.Memories)
	settings.Memories.Enabled = &enabled
	if err := godex.SaveSettings(cfg.DataDir, settings); err != nil {
		return err
	}
	state := "disabled"
	if enabled {
		state = "enabled"
	}
	fmt.Printf("memories\t%s\n", state)
	return nil
}

func parseMemoryCommandConfig(name string, args []string, withLimit bool) (godex.Config, int, error) {
	flags := newFlagSet(name)
	cfg := godex.DefaultConfig()
	limit := memory.DefaultRecallLimit
	bindConfigFlags(flags, &cfg)
	if withLimit {
		flags.IntVar(&limit, "limit", limit, "maximum memory rows")
	}
	if err := flags.Parse(args); err != nil {
		return cfg, limit, err
	}
	loaded, err := loadConfigFromFlags(cfg, flags)
	if err != nil {
		return cfg, limit, err
	}
	return loaded.Config, limit, nil
}

func memoryCommandArgs(args []string) []string {
	flags := newFlagSet("gode memory query")
	cfg := godex.DefaultConfig()
	limit := memory.DefaultRecallLimit
	bindConfigFlags(flags, &cfg)
	flags.IntVar(&limit, "limit", limit, "maximum memory rows")
	_ = flags.Parse(args)
	return flags.Args()
}

func newMemoryCommandService(ctx context.Context, cfg godex.Config) (*memory.Service, func(), error) {
	cfg.Memories = cfg.Memories.WithDefaults(cfg.DataDir)
	scope, err := memory.NewScope(cfg.Workspace, cfg.Memories.DatabasePath, cfg.DataDir)
	if err != nil {
		return nil, nil, err
	}
	store, err := memory.OpenStore(ctx, scope.DatabasePath)
	if err != nil {
		return nil, nil, err
	}
	service := memory.NewService(store, memoryCommandEmbedderFactory(cfg.Memories.EmbeddingModel), scope, cfg.Memories, nil)
	return service, func() {
		_ = service.Close()
	}, nil
}

func memorySettingsFromConfig(cfg memory.Config) memory.Settings {
	enabled := cfg.Enabled
	autoRecall := cfg.AutoRecall
	autoObserve := cfg.AutoObserve
	return memory.Settings{
		Enabled:        &enabled,
		AutoRecall:     &autoRecall,
		AutoObserve:    &autoObserve,
		EmbeddingModel: cfg.EmbeddingModel,
		RecallLimit:    cfg.RecallLimit,
		DatabasePath:   cfg.DatabasePath,
	}
}

func previewMemoryContent(content string) string {
	content = strings.Join(strings.Fields(strings.TrimSpace(content)), " ")
	const max = 96
	if len([]rune(content)) <= max {
		return content
	}
	runes := []rune(content)
	return string(runes[:max-3]) + "..."
}

const timeFormat = "2006-01-02T15:04:05Z07:00"
