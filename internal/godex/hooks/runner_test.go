package hooks

import (
	"context"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"time"
)

func TestRunnerFiltersAndAggregatesDeterministically(t *testing.T) {
	runner := New([]Hook{
		shellHook(t, "unmatched", `printf '{"decision":"deny"}'`, "other"),
		shellHook(t, "allow", `printf '{"decision":"allow","context":"extra context","updated_input":{"command":"go test ./..."}}'`, "shell"),
		shellHook(t, "warning", `echo warn >&2; exit 7`, "shell"),
	})

	result, err := runner.Run(context.Background(), HookInput{
		Tool:      "shell",
		SessionID: "s1",
		Workspace: t.TempDir(),
		Input:     map[string]any{"command": "go test ./cmd/gode", "keep": true},
	})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.Decision != DecisionAllow {
		t.Fatalf("decision = %q", result.Decision)
	}
	if result.Context != "extra context" {
		t.Fatalf("context = %q", result.Context)
	}
	if result.UpdatedInput["command"] != "go test ./..." || result.UpdatedInput["keep"] != true {
		t.Fatalf("updated input = %#v", result.UpdatedInput)
	}
	if len(result.Warnings) != 1 || !strings.Contains(result.Warnings[0], "exited 7") {
		t.Fatalf("warnings = %#v", result.Warnings)
	}
}

func TestRunnerDenyBeatsAllowAndHaltBeatsDeny(t *testing.T) {
	deny := New([]Hook{
		shellHook(t, "allow", `printf '{"decision":"allow"}'`, "*"),
		shellHook(t, "deny", `exit 2`, "*"),
	})
	result, err := deny.Run(context.Background(), HookInput{Tool: "edit", Workspace: t.TempDir(), Input: map[string]any{}})
	if err != nil {
		t.Fatalf("deny run: %v", err)
	}
	if result.Decision != DecisionDeny {
		t.Fatalf("deny decision = %q", result.Decision)
	}

	halt := New([]Hook{
		shellHook(t, "deny", `exit 2`, "*"),
		shellHook(t, "halt", `exit 49`, "*"),
	})
	result, err = halt.Run(context.Background(), HookInput{Tool: "edit", Workspace: t.TempDir(), Input: map[string]any{}})
	if err != nil {
		t.Fatalf("halt run: %v", err)
	}
	if result.Decision != DecisionHalt {
		t.Fatalf("halt decision = %q", result.Decision)
	}
}

func TestRunnerEnvironmentAndInputRewrite(t *testing.T) {
	script := `case "$GODE:$AGENT:$AI_AGENT:$GODE_EVENT:$GODE_TOOL_NAME:$GODE_SESSION_ID:$GODE_CWD:$GODE_TOOL_INPUT_JSON" in *"1:gode:gode:PreToolUse:edit:s1:"*"old"*) printf '{"updated_input":{"new":"value"}}' ;; *) exit 2 ;; esac`
	runner := New([]Hook{shellHook(t, "env", script, "edit")})
	workspace := t.TempDir()
	result, err := runner.Run(context.Background(), HookInput{
		Tool:      "edit",
		SessionID: "s1",
		Workspace: workspace,
		Input:     map[string]any{"old": "value"},
	})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.Decision != DecisionNone {
		t.Fatalf("decision = %q", result.Decision)
	}
	if result.UpdatedInput["old"] != "value" || result.UpdatedInput["new"] != "value" {
		t.Fatalf("updated input = %#v", result.UpdatedInput)
	}
}

func TestRunnerTimeoutAndInvalidJSONAreWarnings(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("shell sleep command is not portable to windows")
	}
	runner := New([]Hook{
		shellHook(t, "bad-json", `printf 'not-json'`, "*"),
		shellHook(t, "slow", `sleep 1`, "*"),
	}, WithDefaultTimeout(10*time.Millisecond))
	result, err := runner.Run(context.Background(), HookInput{Tool: "edit", Workspace: t.TempDir(), Input: map[string]any{}})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if len(result.Warnings) != 2 {
		t.Fatalf("warnings = %#v", result.Warnings)
	}
}

func shellHook(t *testing.T, name string, script string, tools ...string) Hook {
	t.Helper()
	dir := t.TempDir()
	path := filepath.Join(dir, name+".sh")
	if err := os.WriteFile(path, []byte("#!/bin/sh\n"+script+"\n"), 0o700); err != nil {
		t.Fatalf("write hook: %v", err)
	}
	return Hook{Name: name, Command: path, Tools: tools}
}
