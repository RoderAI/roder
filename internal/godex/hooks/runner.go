package hooks

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"strings"
	"time"
)

const (
	exitDeny = 2
	exitHalt = 49
)

type Runner struct {
	hooks          []Hook
	defaultTimeout time.Duration
}

type Option func(*Runner)

func WithDefaultTimeout(timeout time.Duration) Option {
	return func(r *Runner) {
		if timeout > 0 {
			r.defaultTimeout = timeout
		}
	}
}

func New(hooks []Hook, opts ...Option) *Runner {
	r := &Runner{
		hooks:          append([]Hook(nil), hooks...),
		defaultTimeout: 30 * time.Second,
	}
	for _, opt := range opts {
		opt(r)
	}
	return r
}

func (r *Runner) Run(ctx context.Context, input HookInput) (HookResult, error) {
	result := HookResult{Decision: DecisionNone, UpdatedInput: cloneInput(input.Input)}
	for _, hook := range r.matching(input.Tool) {
		outcome := r.runOne(ctx, hook, input, result.UpdatedInput)
		if outcome.Context != "" {
			if result.Context != "" {
				result.Context += "\n"
			}
			result.Context += outcome.Context
		}
		if len(outcome.UpdatedInput) > 0 {
			result.UpdatedInput = mergeInput(result.UpdatedInput, outcome.UpdatedInput)
		}
		result.Warnings = append(result.Warnings, outcome.Warnings...)
		result.Decision = aggregateDecision(result.Decision, outcome.Decision)
	}
	return result, nil
}

func (r *Runner) matching(tool string) []Hook {
	var out []Hook
	for _, hook := range r.hooks {
		if len(hook.Tools) == 0 {
			out = append(out, hook)
			continue
		}
		for _, pattern := range hook.Tools {
			if pattern == "*" || pattern == tool {
				out = append(out, hook)
				break
			}
		}
	}
	return out
}

func (r *Runner) runOne(ctx context.Context, hook Hook, input HookInput, currentInput map[string]any) HookResult {
	timeout := hook.Timeout
	if timeout <= 0 {
		timeout = r.defaultTimeout
	}
	hookCtx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()

	cmd := exec.CommandContext(hookCtx, hook.Command, hook.Args...)
	cmd.Dir = input.Workspace
	cmd.Env = hookEnv(input, currentInput)
	var stdout bytes.Buffer
	var stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	err := cmd.Run()
	if hookCtx.Err() != nil {
		return HookResult{Warnings: []string{fmt.Sprintf("%s timed out: %v", hookName(hook), hookCtx.Err())}}
	}
	parsed, parseErr := parseStdout(stdout.Bytes())
	if parseErr != nil {
		parsed.Warnings = append(parsed.Warnings, fmt.Sprintf("%s stdout parse warning: %v", hookName(hook), parseErr))
	}
	if err == nil {
		return parsed
	}
	var exitErr *exec.ExitError
	if errors.As(err, &exitErr) {
		switch exitErr.ExitCode() {
		case exitDeny:
			parsed.Decision = DecisionDeny
			return parsed
		case exitHalt:
			parsed.Decision = DecisionHalt
			return parsed
		default:
			warning := fmt.Sprintf("%s exited %d", hookName(hook), exitErr.ExitCode())
			if text := strings.TrimSpace(stderr.String()); text != "" {
				warning += ": " + text
			}
			parsed.Warnings = append(parsed.Warnings, warning)
			return parsed
		}
	}
	parsed.Warnings = append(parsed.Warnings, fmt.Sprintf("%s failed: %v", hookName(hook), err))
	return parsed
}

func parseStdout(data []byte) (HookResult, error) {
	text := strings.TrimSpace(string(data))
	if text == "" {
		return HookResult{}, nil
	}
	var payload struct {
		Decision     Decision       `json:"decision"`
		Context      string         `json:"context"`
		UpdatedInput map[string]any `json:"updated_input"`
	}
	if err := json.Unmarshal([]byte(text), &payload); err != nil {
		return HookResult{}, err
	}
	return HookResult{
		Decision:     normalizeDecision(payload.Decision),
		Context:      strings.TrimSpace(payload.Context),
		UpdatedInput: payload.UpdatedInput,
	}, nil
}

func hookEnv(input HookInput, currentInput map[string]any) []string {
	env := append([]string(nil), os.Environ()...)
	data, _ := json.Marshal(currentInput)
	env = append(env,
		"GODE=1",
		"AGENT=gode",
		"AI_AGENT=gode",
		"GODE_EVENT=PreToolUse",
		"GODE_TOOL_NAME="+input.Tool,
		"GODE_SESSION_ID="+input.SessionID,
		"GODE_CWD="+input.Workspace,
		"GODE_TOOL_INPUT_JSON="+string(data),
	)
	return env
}

func aggregateDecision(current Decision, next Decision) Decision {
	next = normalizeDecision(next)
	current = normalizeDecision(current)
	if current == DecisionHalt || next == DecisionHalt {
		return DecisionHalt
	}
	if current == DecisionDeny || next == DecisionDeny {
		return DecisionDeny
	}
	if current == DecisionAllow || next == DecisionAllow {
		return DecisionAllow
	}
	return DecisionNone
}

func normalizeDecision(decision Decision) Decision {
	switch decision {
	case DecisionAllow, DecisionDeny, DecisionHalt:
		return decision
	default:
		return DecisionNone
	}
}

func mergeInput(base map[string]any, patch map[string]any) map[string]any {
	out := cloneInput(base)
	for key, value := range patch {
		out[key] = value
	}
	return out
}

func cloneInput(input map[string]any) map[string]any {
	out := map[string]any{}
	for key, value := range input {
		out[key] = value
	}
	return out
}

func hookName(hook Hook) string {
	if hook.Name != "" {
		return hook.Name
	}
	if hook.Command != "" {
		return hook.Command
	}
	return "hook"
}
