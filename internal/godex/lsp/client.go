package lsp

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
)

type diagnosticHandler func(path string, diagnostics []Diagnostic)

type client struct {
	name      string
	workspace string
	cfg       Config
	cmd       *exec.Cmd
	stdin     io.WriteCloser
	onDiag    diagnosticHandler
	writeMu   sync.Mutex
}

func startClient(ctx context.Context, name string, workspace string, cfg Config, onDiag diagnosticHandler) (*client, error) {
	if strings.TrimSpace(cfg.Command) == "" {
		return nil, fmt.Errorf("missing command")
	}
	cmd := exec.CommandContext(ctx, cfg.Command, cfg.Args...)
	cmd.Env = os.Environ()
	for key, value := range cfg.Env {
		cmd.Env = append(cmd.Env, key+"="+value)
	}
	stdin, err := cmd.StdinPipe()
	if err != nil {
		return nil, err
	}
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return nil, err
	}
	if err := cmd.Start(); err != nil {
		return nil, err
	}
	c := &client{name: name, workspace: workspace, cfg: cfg, cmd: cmd, stdin: stdin, onDiag: onDiag}
	go c.readLoop(stdout)
	_ = c.send(map[string]any{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": map[string]any{"rootUri": pathToURI(workspace)}})
	_ = c.send(map[string]any{"jsonrpc": "2.0", "method": "initialized", "params": map[string]any{}})
	return c, nil
}

func (c *client) close() error {
	if c.stdin != nil {
		_ = c.stdin.Close()
	}
	if c.cmd != nil && c.cmd.Process != nil {
		_ = c.cmd.Process.Kill()
		_, _ = c.cmd.Process.Wait()
	}
	return nil
}

func (c *client) send(message map[string]any) error {
	data, err := json.Marshal(message)
	if err != nil {
		return err
	}
	c.writeMu.Lock()
	defer c.writeMu.Unlock()
	_, err = fmt.Fprintf(c.stdin, "Content-Length: %d\r\n\r\n%s", len(data), data)
	return err
}

func (c *client) readLoop(reader io.Reader) {
	buf := bufio.NewReader(reader)
	for {
		data, err := readMessage(buf)
		if err != nil {
			return
		}
		var msg struct {
			Method string          `json:"method"`
			Params json.RawMessage `json:"params"`
		}
		if err := json.Unmarshal(data, &msg); err != nil || msg.Method != "textDocument/publishDiagnostics" {
			continue
		}
		var params struct {
			URI         string `json:"uri"`
			Diagnostics []struct {
				Range    Range  `json:"range"`
				Severity int    `json:"severity"`
				Message  string `json:"message"`
			} `json:"diagnostics"`
		}
		if err := json.Unmarshal(msg.Params, &params); err != nil {
			continue
		}
		path := uriToPath(params.URI)
		diagnostics := make([]Diagnostic, 0, len(params.Diagnostics))
		for _, item := range params.Diagnostics {
			diagnostics = append(diagnostics, Diagnostic{Server: c.name, Path: path, Range: item.Range, Severity: item.Severity, Message: item.Message})
		}
		c.onDiag(path, diagnostics)
	}
}

func readMessage(reader *bufio.Reader) ([]byte, error) {
	contentLength := 0
	for {
		line, err := reader.ReadString('\n')
		if err != nil {
			return nil, err
		}
		line = strings.TrimRight(line, "\r\n")
		if line == "" {
			break
		}
		key, value, ok := strings.Cut(line, ":")
		if ok && strings.EqualFold(strings.TrimSpace(key), "Content-Length") {
			contentLength, _ = strconv.Atoi(strings.TrimSpace(value))
		}
	}
	if contentLength <= 0 {
		return nil, fmt.Errorf("missing content length")
	}
	data := make([]byte, contentLength)
	_, err := io.ReadFull(reader, data)
	return data, err
}

func writeMessage(writer io.Writer, message map[string]any) error {
	data, err := json.Marshal(message)
	if err != nil {
		return err
	}
	_, err = fmt.Fprintf(writer, "Content-Length: %d\r\n\r\n%s", len(data), data)
	return err
}

func pathToURI(path string) string {
	abs, err := filepath.Abs(path)
	if err != nil {
		abs = path
	}
	return (&url.URL{Scheme: "file", Path: filepath.ToSlash(abs)}).String()
}

func uriToPath(raw string) string {
	parsed, err := url.Parse(raw)
	if err != nil || parsed.Scheme != "file" {
		return raw
	}
	path := parsed.Path
	if path == "" {
		return raw
	}
	if filepath.Separator != '/' {
		path = strings.TrimPrefix(path, "/")
	}
	return filepath.Clean(path)
}
