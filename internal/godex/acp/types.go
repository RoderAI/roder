package acp

import (
	"encoding/json"
	"fmt"
	"path/filepath"
	"strings"
	"time"

	"github.com/pandelisz/gode/internal/godex/mcp"
)

const protocolVersion = 1

type implementation struct {
	Name    string `json:"name"`
	Title   string `json:"title,omitempty"`
	Version string `json:"version,omitempty"`
}

type initializeParams struct {
	ProtocolVersion int            `json:"protocolVersion"`
	ClientInfo      implementation `json:"clientInfo,omitempty"`
}

type newSessionParams struct {
	CWD        string      `json:"cwd"`
	MCPServers []mcpServer `json:"mcpServers"`
}

type listSessionsParams struct {
	CWD    string `json:"cwd"`
	Cursor string `json:"cursor"`
}

type closeSessionParams struct {
	SessionID string `json:"sessionId"`
}

type cancelParams struct {
	SessionID string `json:"sessionId"`
}

type promptParams struct {
	SessionID string            `json:"sessionId"`
	Prompt    []json.RawMessage `json:"prompt"`
}

type contentHeader struct {
	Type string `json:"type"`
}

type textContent struct {
	Text string `json:"text"`
}

type resourceLinkContent struct {
	URI         string `json:"uri"`
	Name        string `json:"name"`
	MimeType    string `json:"mimeType,omitempty"`
	Title       string `json:"title,omitempty"`
	Description string `json:"description,omitempty"`
}

type mcpServer struct {
	Type    string        `json:"type"`
	Name    string        `json:"name"`
	Command string        `json:"command"`
	Args    []string      `json:"args"`
	Env     []envVariable `json:"env"`
}

type envVariable struct {
	Name  string `json:"name"`
	Value string `json:"value"`
}

type sessionInfo struct {
	SessionID string `json:"sessionId"`
	CWD       string `json:"cwd"`
	Title     string `json:"title,omitempty"`
	UpdatedAt string `json:"updatedAt,omitempty"`
}

func initializeResult(version string) map[string]any {
	return map[string]any{
		"protocolVersion": protocolVersion,
		"agentCapabilities": map[string]any{
			"loadSession": false,
			"mcpCapabilities": map[string]any{
				"http": false,
				"sse":  false,
			},
			"promptCapabilities": map[string]any{
				"image":           false,
				"audio":           false,
				"embeddedContext": false,
			},
			"sessionCapabilities": map[string]any{
				"list":  map[string]any{},
				"close": map[string]any{},
			},
		},
		"agentInfo": map[string]any{
			"name":    "gode",
			"title":   "Gode",
			"version": version,
		},
		"authMethods": []any{},
	}
}

func promptToText(blocks []json.RawMessage) (string, error) {
	if len(blocks) == 0 {
		return "", fmt.Errorf("prompt must contain at least one content block")
	}
	parts := make([]string, 0, len(blocks))
	for _, raw := range blocks {
		var header contentHeader
		if err := json.Unmarshal(raw, &header); err != nil {
			return "", err
		}
		switch header.Type {
		case "text":
			var content textContent
			if err := json.Unmarshal(raw, &content); err != nil {
				return "", err
			}
			if strings.TrimSpace(content.Text) == "" {
				return "", fmt.Errorf("text content must not be empty")
			}
			parts = append(parts, content.Text)
		case "resource_link":
			var link resourceLinkContent
			if err := json.Unmarshal(raw, &link); err != nil {
				return "", err
			}
			if strings.TrimSpace(link.URI) == "" {
				return "", fmt.Errorf("resource_link.uri is required")
			}
			if strings.TrimSpace(link.Name) == "" {
				return "", fmt.Errorf("resource_link.name is required")
			}
			parts = append(parts, fmt.Sprintf("Resource link: %s (%s)", link.Name, link.URI))
		case "image", "audio", "resource":
			return "", fmt.Errorf("prompt content type %q is not supported by advertised capabilities", header.Type)
		default:
			return "", fmt.Errorf("unsupported prompt content type %q", header.Type)
		}
	}
	return strings.Join(parts, "\n\n"), nil
}

func validateMCPServers(servers []mcpServer) ([]mcp.ServerConfig, []string, error) {
	configs := make([]mcp.ServerConfig, 0, len(servers))
	names := make([]string, 0, len(servers))
	seen := make(map[string]struct{}, len(servers))
	for _, server := range servers {
		if server.Type != "" && server.Type != "stdio" {
			return nil, nil, fmt.Errorf("mcp server %q uses unsupported transport %q", server.Name, server.Type)
		}
		name := strings.TrimSpace(server.Name)
		if name == "" {
			return nil, nil, fmt.Errorf("mcp server name is required")
		}
		if _, ok := seen[name]; ok {
			return nil, nil, fmt.Errorf("duplicate mcp server %q", name)
		}
		seen[name] = struct{}{}
		if strings.TrimSpace(server.Command) == "" {
			return nil, nil, fmt.Errorf("mcp server %q command is required", name)
		}
		if !filepath.IsAbs(server.Command) {
			return nil, nil, fmt.Errorf("mcp server %q command must be absolute", name)
		}
		env := make(map[string]string, len(server.Env))
		for _, item := range server.Env {
			if strings.TrimSpace(item.Name) == "" {
				return nil, nil, fmt.Errorf("mcp server %q env name is required", name)
			}
			env[item.Name] = item.Value
		}
		configs = append(configs, mcp.ServerConfig{Command: server.Command, Args: server.Args, Env: env})
		names = append(names, name)
	}
	return configs, names, nil
}

func formatACPTime(t time.Time) string {
	if t.IsZero() {
		return ""
	}
	return t.UTC().Format(time.RFC3339)
}

func toolKind(name string) string {
	lower := strings.ToLower(name)
	switch {
	case strings.Contains(lower, "read"), strings.Contains(lower, "list"), strings.Contains(lower, "metadata"):
		return "read"
	case strings.Contains(lower, "write"), strings.Contains(lower, "patch"), strings.Contains(lower, "edit"):
		return "edit"
	case strings.Contains(lower, "remove"), strings.Contains(lower, "delete"):
		return "delete"
	case strings.Contains(lower, "move"), strings.Contains(lower, "copy"):
		return "move"
	case strings.Contains(lower, "search"), strings.Contains(lower, "grep"):
		return "search"
	case strings.Contains(lower, "shell"), strings.Contains(lower, "exec"), strings.Contains(lower, "command"):
		return "execute"
	default:
		return "other"
	}
}
