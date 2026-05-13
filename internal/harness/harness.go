package harness

import (
	"context"
	"fmt"
	"strings"

	"github.com/pandelisz/gode/internal/provider"
)

// Harness is the local orchestration layer for future TUI sessions, tools,
// workspace state, and provider calls.
type Harness struct {
	providers provider.Registry
}

func New(providers provider.Registry) *Harness {
	return &Harness{providers: providers}
}

func (h *Harness) Start(ctx context.Context) error {
	select {
	case <-ctx.Done():
		return ctx.Err()
	default:
	}

	names := h.providers.Names()
	fmt.Printf("gode scaffold ready\nproviders: %s\n", strings.Join(names, ", "))
	return nil
}
