package main

import (
	"context"
	"flag"
	"fmt"
	"os"
	"strings"

	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/tui"
)

var version = "dev"

func main() {
	if err := run(context.Background(), os.Args[1:]); err != nil {
		fmt.Fprintf(os.Stderr, "gode: %v\n", err)
		os.Exit(1)
	}
}

func run(ctx context.Context, args []string) error {
	if len(args) > 0 && args[0] == "version" {
		fmt.Println("gode " + version)
		return nil
	}

	if len(args) > 0 && args[0] == "run" {
		return runPrompt(ctx, args[1:])
	}

	cfg, err := parseConfig(args)
	if err != nil {
		return err
	}
	app, err := godex.New(ctx, cfg)
	if err != nil {
		return err
	}
	defer app.Close(ctx)
	return tui.Run(ctx, app)
}

func runPrompt(ctx context.Context, args []string) error {
	flags := newFlagSet("gode run")
	cfg := godex.DefaultConfig()
	bindConfigFlags(flags, &cfg)
	if err := flags.Parse(args); err != nil {
		return err
	}
	prompt := strings.TrimSpace(strings.Join(flags.Args(), " "))
	if prompt == "" {
		return fmt.Errorf("prompt is required")
	}
	app, err := godex.New(ctx, cfg)
	if err != nil {
		return err
	}
	defer app.Close(ctx)
	result, err := app.RunPrompt(ctx, prompt)
	if err != nil {
		return err
	}
	fmt.Println(result.FinalText)
	return nil
}

func parseConfig(args []string) (godex.Config, error) {
	flags := newFlagSet("gode")
	cfg := godex.DefaultConfig()
	bindConfigFlags(flags, &cfg)
	return cfg, flags.Parse(args)
}

func bindConfigFlags(flags *flag.FlagSet, cfg *godex.Config) {
	flags.StringVar(&cfg.Workspace, "workspace", cfg.Workspace, "workspace root")
	flags.StringVar(&cfg.DataDir, "data-dir", cfg.DataDir, "gode data directory")
	flags.StringVar(&cfg.Provider, "provider", cfg.Provider, "provider: mock, codex, openai")
	flags.StringVar(&cfg.Model, "model", cfg.Model, "provider model")
	flags.StringVar(&cfg.Reasoning, "reasoning", cfg.Reasoning, "reasoning effort: none, minimal, low, medium, high, xhigh")
	flags.BoolVar(&cfg.AutoApprove, "auto-approve", cfg.AutoApprove, "auto approve mutating tool calls")
	flags.BoolVar(&cfg.Telemetry, "telemetry", cfg.Telemetry, "export OpenTelemetry traces over OTLP/gRPC")
	flags.StringVar(&cfg.TelemetryEndpoint, "telemetry-endpoint", cfg.TelemetryEndpoint, "OTLP/gRPC endpoint for traces")
}

func newFlagSet(name string) *flag.FlagSet {
	flags := flag.NewFlagSet(name, flag.ContinueOnError)
	flags.SetOutput(os.Stderr)
	return flags
}
