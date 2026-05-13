package provider

import (
	"context"
	"sort"
)

const (
	CodexName  = "codex"
	ClaudeName = "claude"
)

type Message struct {
	Role    string
	Content string
}

type Response struct {
	Content string
}

type Provider interface {
	Name() string
	Complete(context.Context, []Message) (Response, error)
}

type Registry map[string]Provider

func (r Registry) Names() []string {
	names := make([]string, 0, len(r))
	for name := range r {
		names = append(names, name)
	}
	sort.Strings(names)
	return names
}
