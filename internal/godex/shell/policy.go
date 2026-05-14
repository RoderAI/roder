package shell

import (
	"context"
	"fmt"
	"strings"

	"mvdan.cc/sh/v3/interp"
)

func policyExecHandler(policy *Policy) func(next interp.ExecHandlerFunc) interp.ExecHandlerFunc {
	return func(next interp.ExecHandlerFunc) interp.ExecHandlerFunc {
		return func(ctx context.Context, args []string) error {
			if policy == nil || len(args) == 0 {
				return next(ctx, args)
			}
			name := args[0]
			if reason := strings.TrimSpace(policy.Blocked[name]); reason != "" {
				hc := interp.HandlerCtx(ctx)
				fmt.Fprintf(hc.Stderr, "%s: %s\n", name, reason)
				return interp.ExitStatus(126)
			}
			if !policy.AllowExternal {
				hc := interp.HandlerCtx(ctx)
				fmt.Fprintf(hc.Stderr, "%s: external command blocked by policy\n", name)
				return interp.ExitStatus(126)
			}
			return next(ctx, args)
		}
	}
}
