package main

import (
	"context"
	"fmt"
	"os"

	"github.com/pandelisz/gode/internal/harness"
	"github.com/pandelisz/gode/internal/provider"
)

func main() {
	if err := run(context.Background(), os.Args[1:]); err != nil {
		fmt.Fprintf(os.Stderr, "gode: %v\n", err)
		os.Exit(1)
	}
}

func run(ctx context.Context, args []string) error {
	if len(args) > 0 && args[0] == "version" {
		fmt.Println("gode dev")
		return nil
	}

	agent := harness.New(provider.Registry{
		provider.CodexName:  provider.Placeholder(provider.CodexName),
		provider.ClaudeName: provider.Placeholder(provider.ClaudeName),
	})

	return agent.Start(ctx)
}
