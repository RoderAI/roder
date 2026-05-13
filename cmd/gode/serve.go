package main

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"

	"github.com/BurntSushi/toml"
	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/appserver"
	"github.com/pandelisz/gode/internal/godex/mcp"
)

type serveIO struct {
	stdin  io.Reader
	stdout io.Writer
	stderr io.Writer
}

func runAppServer(ctx context.Context, args []string) error {
	return runServe(ctx, "gode app-server", args)
}

func parseAppServerConfig(args []string) (godex.Config, appserver.ListenConfig, error) {
	return parseServeConfig("gode app-server", args)
}

func runServe(ctx context.Context, command string, args []string) error {
	cfg, listen, err := parseServeConfig(command, args)
	if err != nil {
		return err
	}
	return serveWithConfig(ctx, command, cfg, listen, defaultServeIO())
}

func runRemoteRuntime(ctx context.Context, args []string) error {
	return runRemoteRuntimeWithIO(ctx, args, defaultServeIO())
}

func runRemoteRuntimeWithIO(ctx context.Context, args []string, ioStreams serveIO) error {
	cfg, listen, err := parseRemoteRuntimeConfig(args)
	if err != nil {
		return err
	}
	return serveWithConfig(ctx, "gode remote-runtime", cfg, listen, ioStreams)
}

func defaultServeIO() serveIO {
	return serveIO{stdin: os.Stdin, stdout: os.Stdout, stderr: os.Stderr}
}

func serveWithConfig(ctx context.Context, command string, cfg godex.Config, listen appserver.ListenConfig, ioStreams serveIO) error {
	app, err := godex.New(ctx, cfg)
	if err != nil {
		return err
	}
	defer app.Close(ctx)
	if listen.Kind == appserver.TransportOff {
		return nil
	}

	server := appserver.New(app, appserver.Options{Version: version})
	switch listen.Kind {
	case appserver.TransportStdio:
		return server.ServeStdio(ctx, ioStreams.stdin, ioStreams.stdout)
	case appserver.TransportWebSocket:
		listener, err := server.ListenWebSocket(ctx, listen.Address)
		if err != nil {
			return err
		}
		defer listener.Close(context.Background())
		fmt.Fprintf(ioStreams.stderr, "%s listening on %s\n", command, listener.WebSocketURL())
		<-ctx.Done()
		return ctx.Err()
	default:
		return fmt.Errorf("unsupported serve transport")
	}
}

func parseServeConfig(command string, args []string) (godex.Config, appserver.ListenConfig, error) {
	flags := newFlagSet(command)
	cfg := godex.DefaultConfig()
	listenRaw := "stdio://"
	mcpConfigPath := ""
	flags.StringVar(&listenRaw, "listen", listenRaw, "transport endpoint: stdio://, ws://IP:PORT, or off")
	flags.StringVar(&mcpConfigPath, "mcp-config", mcpConfigPath, "path to an MCP config file")
	bindConfigFlags(flags, &cfg)
	if err := flags.Parse(args); err != nil {
		return cfg, appserver.ListenConfig{}, err
	}
	loaded, err := loadConfigFromFlags(cfg, flags)
	if err != nil {
		return cfg, appserver.ListenConfig{}, err
	}
	cfg = loaded.Config
	if mcpConfigPath != "" {
		if err := applyMCPConfigPath(&cfg, mcpConfigPath); err != nil {
			return cfg, appserver.ListenConfig{}, err
		}
	}
	listen, err := appserver.ParseListenURL(listenRaw)
	if err != nil {
		return cfg, appserver.ListenConfig{}, err
	}
	return cfg, listen, nil
}

func parseRemoteRuntimeConfig(args []string) (godex.Config, appserver.ListenConfig, error) {
	flags := newFlagSet("gode remote-runtime")
	cfg := godex.DefaultConfig()
	listenRaw := "stdio://"
	cwd := ""
	mcpConfigPath := ""
	flags.StringVar(&listenRaw, "listen", listenRaw, "local protocol endpoint: stdio://, ws://IP:PORT, or off")
	flags.StringVar(&cwd, "cwd", cwd, "workspace root to pin for this local runtime")
	flags.StringVar(&mcpConfigPath, "mcp-config", mcpConfigPath, "path to an MCP config file")
	bindConfigFlags(flags, &cfg)
	flags.Usage = func() {
		fmt.Fprintf(flags.Output(), "Usage of %s:\n", flags.Name())
		flags.PrintDefaults()
		fmt.Fprintln(flags.Output(), "Cloud workspace creation, TLS pinning, tunnels, git bootstrap, and machine APIs are outside this local phase.")
	}
	if err := flags.Parse(args); err != nil {
		return cfg, appserver.ListenConfig{}, err
	}
	cwd = strings.TrimSpace(cwd)
	if cwd != "" {
		cfg.Workspace = cwd
	}
	loaded, err := loadConfigFromFlags(cfg, flags)
	if err != nil {
		return cfg, appserver.ListenConfig{}, err
	}
	cfg = loaded.Config
	if cwd != "" {
		cfg.Workspace = cwd
	}
	if mcpConfigPath != "" {
		if err := applyMCPConfigPath(&cfg, mcpConfigPath); err != nil {
			return cfg, appserver.ListenConfig{}, err
		}
	}
	listen, err := appserver.ParseListenURL(listenRaw)
	if err != nil {
		return cfg, appserver.ListenConfig{}, err
	}
	return cfg, listen, nil
}

func applyMCPConfigPath(cfg *godex.Config, path string) error {
	servers, err := loadMCPConfigFile(path)
	if err != nil {
		return err
	}
	if cfg.MCP == nil {
		cfg.MCP = map[string]mcp.ServerConfig{}
	}
	for name, server := range servers {
		cfg.MCP[name] = server
	}
	return nil
}

type mcpConfigFile struct {
	MCP map[string]any `json:"mcp" toml:"mcp"`
}

func loadMCPConfigFile(path string) (map[string]mcp.ServerConfig, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read mcp config: %w", err)
	}
	ext := strings.ToLower(filepath.Ext(path))
	var wrapper mcpConfigFile
	var raw map[string]any
	switch ext {
	case ".json":
		if err := json.Unmarshal(data, &wrapper); err != nil {
			return nil, fmt.Errorf("parse mcp config: %w", err)
		}
		if wrapper.MCP == nil {
			if err := json.Unmarshal(data, &raw); err != nil {
				return nil, fmt.Errorf("parse mcp config: %w", err)
			}
		}
	case ".toml":
		if err := toml.Unmarshal(data, &wrapper); err != nil {
			return nil, fmt.Errorf("parse mcp config: %w", err)
		}
		if wrapper.MCP == nil {
			if err := toml.Unmarshal(data, &raw); err != nil {
				return nil, fmt.Errorf("parse mcp config: %w", err)
			}
		}
	default:
		return nil, fmt.Errorf("parse mcp config: unsupported extension")
	}
	if wrapper.MCP != nil {
		raw = wrapper.MCP
	}
	return mcp.ParseConfigMap(raw)
}
