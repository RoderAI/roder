package shell

import (
	"context"
	"fmt"
	"io"
	"sort"
	"strings"

	"mvdan.cc/sh/v3/interp"
)

type BuiltinFunc func(ctx context.Context, args []string, stdin io.Reader, stdout io.Writer, stderr io.Writer) error

type Builtin struct {
	Name        string
	Description string
	ReadOnly    bool
	Run         BuiltinFunc
}

type BuiltinRegistry struct {
	items map[string]Builtin
}

func NewBuiltinRegistry() *BuiltinRegistry {
	return &BuiltinRegistry{items: map[string]Builtin{}}
}

func (r *BuiltinRegistry) Register(builtin Builtin) error {
	if r == nil {
		return fmt.Errorf("builtin registry is nil")
	}
	name := strings.TrimSpace(builtin.Name)
	if name == "" {
		return fmt.Errorf("builtin name is required")
	}
	if builtin.Run == nil {
		return fmt.Errorf("builtin %s run function is required", name)
	}
	if r.items == nil {
		r.items = map[string]Builtin{}
	}
	if _, exists := r.items[name]; exists {
		return fmt.Errorf("builtin %s already registered", name)
	}
	builtin.Name = name
	r.items[name] = builtin
	return nil
}

func (r *BuiltinRegistry) Lookup(name string) (Builtin, bool) {
	if r == nil {
		return Builtin{}, false
	}
	builtin, ok := r.items[name]
	return builtin, ok
}

func (r *BuiltinRegistry) List() []Builtin {
	if r == nil {
		return nil
	}
	names := make([]string, 0, len(r.items))
	for name := range r.items {
		names = append(names, name)
	}
	sort.Strings(names)
	out := make([]Builtin, 0, len(names))
	for _, name := range names {
		out = append(out, r.items[name])
	}
	return out
}

func builtinExecHandler(reg *BuiltinRegistry) func(next interp.ExecHandlerFunc) interp.ExecHandlerFunc {
	return func(next interp.ExecHandlerFunc) interp.ExecHandlerFunc {
		return func(ctx context.Context, args []string) error {
			if len(args) == 0 {
				return next(ctx, args)
			}
			builtin, ok := reg.Lookup(args[0])
			if !ok {
				return next(ctx, args)
			}
			hc := interp.HandlerCtx(ctx)
			return builtin.Run(ctx, args, hc.Stdin, hc.Stdout, hc.Stderr)
		}
	}
}
