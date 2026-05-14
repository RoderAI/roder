package main

import (
	"context"
	"crypto/rand"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"time"

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

	remoteOptions := appserver.RemoteOptions{}
	remoteToken := ""
	if listen.Remote.Enabled {
		token, auth, err := prepareRemoteAuth(listen.Remote.AuthToken)
		if err != nil {
			return err
		}
		remoteToken = token
		remoteOptions = appserver.RemoteOptions{
			Enabled:        true,
			Auth:           auth,
			AllowedOrigins: listen.Remote.AllowedOrigins,
			ServerName:     "Gode Remote",
		}
	}

	server := appserver.New(app, appserver.Options{Version: version, Remote: remoteOptions, Log: ioStreams.stderr})
	switch listen.Kind {
	case appserver.TransportStdio:
		return server.ServeStdio(ctx, ioStreams.stdin, ioStreams.stdout)
	case appserver.TransportWebSocket:
		listener, err := server.ListenWebSocket(ctx, listen.Address)
		if err != nil {
			return err
		}
		defer listener.Close(context.Background())
		if listen.Remote.Enabled {
			if err := printRemoteServerInfo(ioStreams.stderr, command, cfg.Workspace, listener, remoteOptions.Auth, remoteToken, listen.Remote.PrintQR); err != nil {
				return err
			}
		} else {
			fmt.Fprintf(ioStreams.stderr, "%s listening on %s\n", command, listener.WebSocketURL())
		}
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
	remote := false
	authToken := ""
	printQR := true
	var allowedOrigins commaListFlag
	flags.StringVar(&listenRaw, "listen", listenRaw, "transport endpoint: stdio://, ws://IP:PORT, or off")
	flags.StringVar(&mcpConfigPath, "mcp-config", mcpConfigPath, "path to an MCP config file")
	flags.BoolVar(&remote, "remote", remote, "serve a remote websocket app-server with bearer authentication")
	flags.StringVar(&authToken, "auth-token", authToken, "remote auth token literal or env:NAME")
	flags.BoolVar(&printQR, "print-qr", printQR, "print a terminal QR code for remote pairing")
	flags.Var(&allowedOrigins, "allowed-origin", "allowed websocket Origin; may be repeated or comma-separated")
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
	visited := visitedFlags(flags)
	if remote && !visited["listen"] {
		listenRaw = "ws://0.0.0.0:0"
	}
	listen, err := appserver.ParseListenURL(listenRaw)
	if err != nil {
		return cfg, appserver.ListenConfig{}, err
	}
	if remote {
		if listen.Kind != appserver.TransportWebSocket {
			return cfg, appserver.ListenConfig{}, fmt.Errorf("--remote requires a websocket listen transport")
		}
		resolvedToken, err := resolveRemoteAuthToken(authToken)
		if err != nil {
			return cfg, appserver.ListenConfig{}, err
		}
		listen.Remote = appserver.RemoteListenConfig{
			Enabled:        true,
			AuthToken:      resolvedToken,
			PrintQR:        printQR,
			AllowedOrigins: allowedOrigins,
		}
	}
	return cfg, listen, nil
}

func prepareRemoteAuth(authToken string) (string, appserver.RemoteAuth, error) {
	token := strings.TrimSpace(authToken)
	if token == "" {
		generated, err := appserver.GenerateRemoteToken(rand.Reader)
		if err != nil {
			return "", appserver.RemoteAuth{}, err
		}
		token = generated.Token
	}
	auth, err := appserver.NewRemoteAuth(token, time.Now())
	if err != nil {
		return "", appserver.RemoteAuth{}, err
	}
	return token, auth, nil
}

func printRemoteServerInfo(stderr io.Writer, command, workspace string, listener *appserver.WebSocketListener, auth appserver.RemoteAuth, token string, printQR bool) error {
	urls := appserver.DiscoverRemoteConnectURLs(listener.Address())
	if len(urls) == 0 {
		fallbackURL := listener.WebSocketURL()
		if fallbackURL == "" {
			fallbackURL = "ws://127.0.0.1:0"
		}
		urls = []string{fallbackURL}
	}
	fmt.Fprintf(stderr, "%s remote app-server listening\n", command)
	for _, remoteURL := range urls {
		fmt.Fprintf(stderr, "  %s\n", remoteURL)
	}
	fmt.Fprintf(stderr, "token preview: %s\n", auth.TokenPreview)
	fmt.Fprintln(stderr, "auth: Authorization: Bearer <token> or websocket subprotocol bearer.<token>")
	payload := appserver.BuildRemotePairingPayload("Gode Remote", urls[0], token, workspace)
	link, err := appserver.RemoteDeepLink(payload)
	if err != nil {
		return err
	}
	fmt.Fprintf(stderr, "pairing url (sensitive): %s\n", link)
	if !printQR {
		return nil
	}
	qr, err := appserver.RenderTerminalQR(link)
	if err != nil {
		return err
	}
	fmt.Fprintln(stderr)
	fmt.Fprintln(stderr, qr)
	fmt.Fprintln(stderr)
	fmt.Fprintln(stderr, "scan the QR code with a Gode remote client to connect")
	return nil
}

func resolveRemoteAuthToken(raw string) (string, error) {
	raw = strings.TrimSpace(raw)
	if raw == "" {
		return "", nil
	}
	name, ok := strings.CutPrefix(raw, "env:")
	if !ok {
		return raw, nil
	}
	value := strings.TrimSpace(os.Getenv(name))
	if value == "" {
		return "", fmt.Errorf("remote auth token env %s is empty", name)
	}
	return value, nil
}

type commaListFlag []string

func (l *commaListFlag) String() string {
	if l == nil {
		return ""
	}
	return strings.Join(*l, ",")
}

func (l *commaListFlag) Set(value string) error {
	for _, part := range strings.Split(value, ",") {
		part = strings.TrimSpace(part)
		if part != "" {
			*l = append(*l, part)
		}
	}
	return nil
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
