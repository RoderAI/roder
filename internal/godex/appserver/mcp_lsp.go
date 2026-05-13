package appserver

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
)

type mcpResourceReadParams struct {
	Server string `json:"server"`
	URI    string `json:"uri"`
}

type lspDiagnosticsParams struct {
	Path string `json:"path"`
}

func (s *Server) handleMCPState() any {
	if s.app == nil || s.app.MCP == nil {
		return map[string]any{"servers": []any{}}
	}
	return map[string]any{"servers": s.app.MCP.States()}
}

func (s *Server) handleMCPResourcesList() any {
	if s.app == nil || s.app.MCP == nil {
		return map[string]any{"resources": []any{}}
	}
	resources := s.app.MCP.Resources()
	if resources == nil {
		return map[string]any{"resources": []any{}}
	}
	return map[string]any{"resources": resources}
}

func (s *Server) handleMCPResourceRead(ctx context.Context, raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[mcpResourceReadParams](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	if strings.TrimSpace(params.Server) == "" {
		return nil, rpcError(errorInvalidParams, "server is required")
	}
	if strings.TrimSpace(params.URI) == "" {
		return nil, rpcError(errorInvalidParams, "uri is required")
	}
	if s.app == nil || s.app.MCP == nil {
		return nil, rpcError(errorInternal, "mcp is not available")
	}
	text, err := s.app.MCP.ReadResource(ctx, params.Server, params.URI)
	if err != nil {
		return nil, rpcError(errorInternal, fmt.Sprintf("read mcp resource: %v", err))
	}
	return map[string]any{"text": text}, nil
}

func (s *Server) handleLSPState() any {
	if s.app == nil || s.app.LSP == nil {
		return map[string]any{"servers": []any{}}
	}
	return map[string]any{"servers": s.app.LSP.States()}
}

func (s *Server) handleLSPDiagnostics(ctx context.Context, raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[lspDiagnosticsParams](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	if strings.TrimSpace(params.Path) == "" {
		return nil, rpcError(errorInvalidParams, "path is required")
	}
	if s.app == nil || s.app.LSP == nil {
		return nil, rpcError(errorInternal, "lsp is not available")
	}
	diagnostics, err := s.app.LSP.Diagnostics(ctx, params.Path)
	if err != nil {
		return nil, rpcError(errorInternal, fmt.Sprintf("lsp diagnostics: %v", err))
	}
	if diagnostics == nil {
		return map[string]any{"diagnostics": []any{}}, nil
	}
	return map[string]any{"diagnostics": diagnostics}, nil
}
