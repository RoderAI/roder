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
	"github.com/pandelisz/gode/internal/godex/appserver"
	"github.com/pandelisz/gode/internal/godex/codexauth"
	"github.com/pandelisz/gode/internal/godex/configstore"
	"github.com/pandelisz/gode/internal/godex/provider"
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

	if len(args) > 0 && args[0] == "auth" {
		return runAuth(ctx, args[1:])
	}

	if len(args) > 0 && args[0] == "acp" {
		return runACP(ctx, args[1:])
	}

	if len(args) > 0 && args[0] == "app-server" {
		return runAppServer(ctx, args[1:])
	}

	if len(args) > 0 && args[0] == "dirs" {
		return runDirs(args[1:])
	}

	if len(args) > 0 && args[0] == "models" {
		return runModels(args[1:])
	}

	if len(args) > 0 && args[0] == "config" {
		return runConfig(args[1:])
	}

	if len(args) > 0 && args[0] == "session" {
		return runSession(args[1:])
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

func runAppServer(ctx context.Context, args []string) error {
	cfg, listen, err := parseAppServerConfig(args)
	if err != nil {
		return err
	}
	if listen.Kind == appserver.TransportOff {
		return nil
	}
	app, err := godex.New(ctx, cfg)
	if err != nil {
		return err
	}
	defer app.Close(ctx)

	server := appserver.New(app, appserver.Options{Version: version})
	switch listen.Kind {
	case appserver.TransportStdio:
		return server.ServeStdio(ctx, os.Stdin, os.Stdout)
	case appserver.TransportWebSocket:
		listener, err := server.ListenWebSocket(ctx, listen.Address)
		if err != nil {
			return err
		}
		defer listener.Close(context.Background())
		fmt.Fprintf(os.Stderr, "gode app-server listening on %s\n", listener.WebSocketURL())
		<-ctx.Done()
		return ctx.Err()
	default:
		return fmt.Errorf("unsupported app-server transport")
	}
}

func runPrompt(ctx context.Context, args []string) error {
	flags := newFlagSet("gode run")
	cfg := godex.DefaultConfig()
	sessionID := ""
	resume := false
	flags.StringVar(&sessionID, "session", sessionID, "session id to use")
	flags.BoolVar(&resume, "resume", resume, "resume prior session messages")
	bindConfigFlags(flags, &cfg)
	if err := flags.Parse(args); err != nil {
		return err
	}
	loaded, err := loadConfigFromFlags(cfg, flags)
	if err != nil {
		return err
	}
	cfg = loaded.Config
	prompt := strings.TrimSpace(strings.Join(flags.Args(), " "))
	if prompt == "" {
		return fmt.Errorf("prompt is required")
	}
	app, err := godex.New(ctx, cfg)
	if err != nil {
		return err
	}
	defer app.Close(ctx)
	var result agent.RunResult
	if sessionID != "" || resume {
		runResult, err := app.Run(ctx, agent.RunRequest{SessionID: sessionID, Prompt: prompt, Resume: resume})
		if err != nil {
			return err
		}
		result = runResult
	} else {
		runResult, err := app.RunPrompt(ctx, prompt)
		if err != nil {
			return err
		}
		result = runResult
	}
	fmt.Println(result.FinalText)
	return nil
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

func parseAppServerConfig(args []string) (godex.Config, appserver.ListenConfig, error) {
	flags := newFlagSet("gode app-server")
	cfg := godex.DefaultConfig()
	listenRaw := "stdio://"
	flags.StringVar(&listenRaw, "listen", listenRaw, "transport endpoint: stdio://, ws://IP:PORT, or off")
	bindConfigFlags(flags, &cfg)
	if err := flags.Parse(args); err != nil {
		return cfg, appserver.ListenConfig{}, err
	}
	loaded, err := loadConfigFromFlags(cfg, flags)
	if err != nil {
		return cfg, appserver.ListenConfig{}, err
	}
	cfg = loaded.Config
	listen, err := appserver.ParseListenURL(listenRaw)
	if err != nil {
		return cfg, appserver.ListenConfig{}, err
	}
	return cfg, listen, nil
}

func bindConfigFlags(flags *flag.FlagSet, cfg *godex.Config) {
	flags.StringVar(&cfg.Workspace, "workspace", cfg.Workspace, "workspace root")
	flags.StringVar(&cfg.DataDir, "data-dir", cfg.DataDir, "gode data directory")
	flags.StringVar(&cfg.Provider, "provider", cfg.Provider, "provider: mock, codex, openai")
	flags.StringVar(&cfg.Model, "model", cfg.Model, "provider model")
	flags.StringVar(&cfg.Reasoning, "reasoning", cfg.Reasoning, "reasoning effort: none, minimal, low, medium, high, xhigh")
	flags.BoolVar(&cfg.FastMode, "fast-mode", cfg.FastMode, "use OpenAI priority processing service tier")
	flags.BoolVar(&cfg.AutoApprove, "auto-approve", cfg.AutoApprove, "auto approve mutating tool calls")
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
		fmt.Printf("%s\t%s\t%s\t%s\n", model.Provider, model.ID, model.DisplayName, model.DefaultReasoning)
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
