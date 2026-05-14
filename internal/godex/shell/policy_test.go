package shell

import (
	"context"
	"fmt"
	"io"
	"strings"
	"testing"
	"time"
)

func TestPolicyDisallowExternalStillAllowsBuiltins(t *testing.T) {
	reg := NewBuiltinRegistry()
	if err := reg.Register(Builtin{
		Name: "gode_echo",
		Run: func(_ context.Context, _ []string, _ io.Reader, stdout io.Writer, _ io.Writer) error {
			fmt.Fprint(stdout, "builtin ok")
			return nil
		},
	}); err != nil {
		t.Fatalf("register: %v", err)
	}

	allowed, err := Runner{Builtins: reg}.Run(context.Background(), RunRequest{
		Command: "gode_echo",
		Policy:  &Policy{AllowExternal: false},
	})
	if err != nil {
		t.Fatalf("run builtin: %v", err)
	}
	if allowed.ExitCode != 0 || allowed.Stdout != "builtin ok" {
		t.Fatalf("allowed = %#v", allowed)
	}

	blocked, err := Runner{Builtins: reg}.Run(context.Background(), RunRequest{
		Command: "env",
		Policy:  &Policy{AllowExternal: false},
	})
	if err != nil {
		t.Fatalf("run external: %v", err)
	}
	if blocked.ExitCode != 126 || !strings.Contains(blocked.Stderr, "external command blocked") {
		t.Fatalf("blocked = %#v", blocked)
	}
}

func TestPolicyBlockedExternalCommandReturns126(t *testing.T) {
	result, err := Runner{}.Run(context.Background(), RunRequest{
		Command: "env",
		Policy: &Policy{
			AllowExternal: true,
			Blocked:       map[string]string{"env": "env is not allowed"},
		},
	})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.ExitCode != 126 || !strings.Contains(result.Stderr, "env is not allowed") {
		t.Fatalf("result = %#v", result)
	}
}

func TestPolicyUnknownExternalCommandReturns127(t *testing.T) {
	result, err := Runner{}.Run(context.Background(), RunRequest{
		Command: "definitely_missing_gode_command",
		Policy:  &Policy{AllowExternal: true},
	})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.ExitCode != 127 {
		t.Fatalf("result = %#v", result)
	}
}

func TestPolicySeesExpandedArgs(t *testing.T) {
	result, err := Runner{}.Run(context.Background(), RunRequest{
		Command: `cmd=env; "$cmd"`,
		Policy: &Policy{
			AllowExternal: true,
			Blocked:       map[string]string{"env": "expanded env blocked"},
		},
	})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.ExitCode != 126 || !strings.Contains(result.Stderr, "expanded env blocked") {
		t.Fatalf("result = %#v", result)
	}
}

func TestPolicyPreservesContextCancellation(t *testing.T) {
	start := time.Now()
	result, err := Runner{}.Run(context.Background(), RunRequest{
		Command: "sleep 5",
		Policy:  &Policy{AllowExternal: true},
		Timeout: 50 * time.Millisecond,
	})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.ExitCode != -1 || result.Stderr == "" {
		t.Fatalf("result = %#v", result)
	}
	if elapsed := time.Since(start); elapsed > time.Second {
		t.Fatalf("timeout took too long: %s", elapsed)
	}
}
