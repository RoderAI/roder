package main

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"strings"

	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/acp"
	"github.com/pandelisz/gode/internal/godex/agent"
	"github.com/pandelisz/gode/internal/godex/codexauth"
	"github.com/pandelisz/gode/internal/godex/configstore"
	"github.com/pandelisz/gode/internal/godex/provider"
	"github.com/pandelisz/gode/internal/tui"
)

var version = "dev"
var pickResumeSession = tui.PickResumeSession
var runResumeTUI = tui.RunSession

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

	if len(args) > 0 && args[0] == "auth" {
		return runAuth(ctx, args[1:])
	}

	if len(args) > 0 && args[0] == "acp" {
		return runACP(ctx, args[1:])
	}

	if len(args) > 0 && args[0] == "serve" {
		return runServe(ctx, "gode serve", args[1:])
	}

	if len(args) > 0 && args[0] == "app-server" {
		return runAppServer(ctx, args[1:])
	}

	if len(args) > 0 && args[0] == "dirs" {
		return runDirs(args[1:])
	}

	if len(args) > 0 && args[0] == "debug" {
		return runDebug(args[1:])
	}

	if len(args) > 0 && args[0] == "models" {
		return runModels(args[1:])
	}

	if len(args) > 0 && args[0] == "memory" {
		return runMemory(ctx, args[1:])
	}

	if len(args) > 0 && args[0] == "config" {
		return runConfig(args[1:])
	}

	if len(args) > 0 && args[0] == "session" {
		return runSession(args[1:])
	}

	if len(args) > 0 && args[0] == "resume" {
		return runResume(ctx, args[1:])
	}

	if len(args) > 0 && args[0] == "goal" {
		return runGoal(ctx, args[1:])
	}

	if len(args) > 0 && args[0] == "skills" {
		return runSkills(ctx, args[1:])
	}

	if len(args) > 0 && !strings.HasPrefix(args[0], "-") {
		return fmt.Errorf("unknown command %q", args[0])
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

func runResume(ctx context.Context, args []string) error {
	cfg, err := parseConfigWithName("gode resume", args)
	if err != nil {
		return err
	}
	app, err := godex.New(ctx, cfg)
	if err != nil {
		return err
	}
	defer app.Close(ctx)
	sessionID, err := pickResumeSession(ctx, app)
	if err != nil {
		return err
	}
	if sessionID == "" {
		return nil
	}
	return runResumeTUI(ctx, app, sessionID)
}

func runAuth(ctx context.Context, args []string) error {
	if len(args) == 0 {
		return fmt.Errorf("usage: gode auth login codex|status|logout")
	}
	switch args[0] {
	case "login":
		flags := newFlagSet("gode auth login")
		cfg := godex.DefaultConfig()
		flags.StringVar(&cfg.DataDir, "data-dir", cfg.DataDir, "gode data directory")
		if err := flags.Parse(args[1:]); err != nil {
			return err
		}
		provider := "codex"
		if flags.NArg() > 0 {
			provider = flags.Arg(0)
		}
		if provider != "codex" {
			return fmt.Errorf("unsupported auth provider %q", provider)
		}
		fmt.Fprintln(os.Stderr, "Opening browser for Codex sign-in...")
		tokens, url, err := codexauth.LoginBrowser(ctx, cfg.DataDir)
		if err != nil {
			if url != "" {
				fmt.Fprintf(os.Stderr, "Codex sign-in URL: %s\n", url)
			}
			return err
		}
		if tokens.AccountID != "" {
			fmt.Fprintf(os.Stderr, "Signed in with Codex account %s\n", tokens.AccountID)
		} else {
			fmt.Fprintln(os.Stderr, "Signed in with Codex")
		}
		return nil
	case "status":
		cfg := godex.DefaultConfig()
		tokens, err := (codexauth.Store{DataDir: cfg.DataDir}).Load()
		if err != nil {
			return err
		}
		if tokens.Refresh == "" {
			fmt.Println("codex: signed out")
			return nil
		}
		if tokens.AccountID != "" {
			fmt.Printf("codex: signed in (%s)\n", tokens.AccountID)
		} else {
			fmt.Println("codex: signed in")
		}
		return nil
	case "logout":
		cfg := godex.DefaultConfig()
		if err := (codexauth.Store{DataDir: cfg.DataDir}).Delete(); err != nil {
			return err
		}
		fmt.Println("codex: signed out")
		return nil
	default:
		return fmt.Errorf("unknown auth command %q", args[0])
	}
}

func runACP(ctx context.Context, args []string) error {
	cfg, err := parseConfigWithName("gode acp", args)
	if err != nil {
		return err
	}
	app, err := godex.New(ctx, cfg)
	if err != nil {
		return err
	}
	defer app.Close(ctx)
	return acp.New(app, acp.Options{Version: version}).ServeStdio(ctx, os.Stdin, os.Stdout)
}

func runPrompt(ctx context.Context, args []string) error {
	flags := newFlagSet("gode run")
	cfg := godex.DefaultConfig()
	sessionID := ""
	resume := false
	promptFlag := ""
	jsonOutput := false
	systemPromptFile := ""
	responseFormat := ""
	mcpConfigPath := ""
	flags.StringVar(&sessionID, "session", sessionID, "session id to use")
	flags.BoolVar(&resume, "resume", resume, "resume prior session messages")
	flags.StringVar(&promptFlag, "prompt", promptFlag, "prompt to run")
	flags.BoolVar(&jsonOutput, "json", jsonOutput, "print a structured JSON result")
	flags.StringVar(&systemPromptFile, "system-prompt-file", systemPromptFile, "path to a system prompt file")
	flags.StringVar(&responseFormat, "response-format", responseFormat, "JSON response format passed to the provider")
	flags.StringVar(&mcpConfigPath, "mcp-config", mcpConfigPath, "path to an MCP config file")
	bindConfigFlags(flags, &cfg)
	if err := flags.Parse(args); err != nil {
		return err
	}
	loaded, err := loadConfigFromFlags(cfg, flags)
	if err != nil {
		return err
	}
	cfg = loaded.Config
	if mcpConfigPath != "" {
		if err := applyMCPConfigPath(&cfg, mcpConfigPath); err != nil {
			return err
		}
	}
	responseFormat = strings.TrimSpace(responseFormat)
	if responseFormat != "" && !json.Valid([]byte(responseFormat)) {
		return fmt.Errorf("response format must be valid JSON")
	}
	instructions := ""
	if systemPromptFile != "" {
		data, err := os.ReadFile(systemPromptFile)
		if err != nil {
			return fmt.Errorf("read system prompt file: %w", err)
		}
		instructions = strings.TrimSpace(string(data))
	}
	prompt := strings.TrimSpace(promptFlag)
	if prompt == "" {
		prompt = strings.TrimSpace(strings.Join(flags.Args(), " "))
	} else if flags.NArg() > 0 {
		prompt = strings.TrimSpace(prompt + " " + strings.Join(flags.Args(), " "))
	}
	if prompt == "" {
		return fmt.Errorf("prompt is required")
	}
	app, err := godex.New(ctx, cfg)
	if err != nil {
		return err
	}
	defer app.Close(ctx)
	result, err := app.Run(ctx, agent.RunRequest{
		SessionID:      sessionID,
		Prompt:         prompt,
		Resume:         resume,
		Instructions:   instructions,
		ResponseFormat: responseFormat,
	})
	if err != nil {
		return err
	}
	if jsonOutput {
		encoder := json.NewEncoder(os.Stdout)
		return encoder.Encode(runJSONOutput{
			SessionID: result.SessionID,
			RunID:     result.RunID,
			FinalText: result.FinalText,
			Model:     cfg.Model,
			Provider:  godex.DisplayProvider(cfg),
		})
	}
	fmt.Println(result.FinalText)
	return nil
}

type runJSONOutput struct {
	SessionID string `json:"session_id"`
	RunID     string `json:"run_id"`
	FinalText string `json:"final_text"`
	Model     string `json:"model"`
	Provider  string `json:"provider"`
}

func parseConfig(args []string) (godex.Config, error) {
	return parseConfigWithName("gode", args)
}

func parseConfigWithName(name string, args []string) (godex.Config, error) {
	flags := newFlagSet("gode")
	if name != "" {
		flags = newFlagSet(name)
	}
	cfg := godex.DefaultConfig()
	bindConfigFlags(flags, &cfg)
	if err := flags.Parse(args); err != nil {
		return cfg, err
	}
	loaded, err := loadConfigFromFlags(cfg, flags)
	if err != nil {
		return cfg, err
	}
	return loaded.Config, nil
}

func bindConfigFlags(flags *flag.FlagSet, cfg *godex.Config) {
	flags.StringVar(&cfg.Workspace, "workspace", cfg.Workspace, "workspace root")
	flags.StringVar(&cfg.DataDir, "data-dir", cfg.DataDir, "gode data directory")
	flags.StringVar(&cfg.Provider, "provider", cfg.Provider, "provider: mock, codex, openai, anthropic")
	flags.StringVar(&cfg.Model, "model", cfg.Model, "provider model")
	flags.StringVar(&cfg.Reasoning, "reasoning", cfg.Reasoning, "reasoning effort: none, minimal, low, medium, high, xhigh")
	flags.BoolVar(&cfg.FastMode, "fast-mode", cfg.FastMode, "use OpenAI priority processing service tier")
	flags.BoolVar(&cfg.AutoApprove, "auto-approve", cfg.AutoApprove, "auto approve mutating tool calls")
	flags.BoolVar(&cfg.DisableAutoCompaction, "disable-auto-compaction", cfg.DisableAutoCompaction, "disable OpenAI Responses server-side compaction")
	flags.IntVar(&cfg.AutoCompactTokenLimit, "auto-compact-token-limit", cfg.AutoCompactTokenLimit, "override OpenAI Responses compaction token threshold")
	flags.BoolVar(&cfg.Telemetry, "telemetry", cfg.Telemetry, "export OpenTelemetry traces over OTLP/gRPC")
	flags.StringVar(&cfg.TelemetryEndpoint, "telemetry-endpoint", cfg.TelemetryEndpoint, "OTLP/gRPC endpoint for traces")
}

func newFlagSet(name string) *flag.FlagSet {
	flags := flag.NewFlagSet(name, flag.ContinueOnError)
	flags.SetOutput(os.Stderr)
	return flags
}

func loadConfigFromFlags(cfg godex.Config, flags *flag.FlagSet) (configstore.Loaded, error) {
	return configstore.Load(configstore.LoadOptions{
		Workspace: cfg.Workspace,
		DataDir:   cfg.DataDir,
		Flags:     cfg,
		FlagSet:   visitedFlags(flags),
	})
}

func visitedFlags(flags *flag.FlagSet) map[string]bool {
	out := map[string]bool{}
	flags.Visit(func(f *flag.Flag) {
		out[f.Name] = true
	})
	return out
}

func runDirs(args []string) error {
	flags := newFlagSet("gode dirs")
	cfg := godex.DefaultConfig()
	bindConfigFlags(flags, &cfg)
	if err := flags.Parse(args); err != nil {
		return err
	}
	loaded, err := loadConfigFromFlags(cfg, flags)
	if err != nil {
		return err
	}
	fmt.Printf("workspace\t%s\n", loaded.Config.Workspace)
	fmt.Printf("data_dir\t%s\n", loaded.Config.DataDir)
	for _, path := range loaded.Paths {
		fmt.Printf("config\t%s\n", path)
	}
	return nil
}

func runModels(args []string) error {
	flags := newFlagSet("gode models")
	cfg := godex.DefaultConfig()
	bindConfigFlags(flags, &cfg)
	if err := flags.Parse(args); err != nil {
		return err
	}
	if _, err := loadConfigFromFlags(cfg, flags); err != nil {
		return err
	}
	for _, model := range provider.Catalog.Models(false) {
		cfg := godex.ModelConfigFor(model.ID)
		fmt.Printf("%s\t%s\t%s\t%s\tcontext=%d\tauto_compact=%d\n", model.Provider, model.ID, model.DisplayName, model.DefaultReasoning, cfg.ContextWindow, cfg.AutoCompactTokenLimit)
	}
	return nil
}

func runConfig(args []string) error {
	if len(args) == 0 || args[0] != "schema" {
		return fmt.Errorf("usage: gode config schema")
	}
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetIndent("", "  ")
	return encoder.Encode(configstore.Schema())
}
