package shell

import (
	"context"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"mvdan.cc/sh/v3/interp"
)

func TestBuiltinRegistryRejectsDuplicateNames(t *testing.T) {
	reg := NewBuiltinRegistry()
	if err := reg.Register(Builtin{Name: "gode_echo", Run: func(context.Context, []string, io.Reader, io.Writer, io.Writer) error { return nil }}); err != nil {
		t.Fatalf("register first: %v", err)
	}
	if err := reg.Register(Builtin{Name: "gode_echo", Run: func(context.Context, []string, io.Reader, io.Writer, io.Writer) error { return nil }}); err == nil {
		t.Fatal("expected duplicate registration error")
	}
}

func TestBuiltinExecHandlerMatchesCommandName(t *testing.T) {
	reg := NewBuiltinRegistry()
	if err := reg.Register(Builtin{
		Name: "upper",
		Run: func(_ context.Context, args []string, _ io.Reader, stdout io.Writer, _ io.Writer) error {
			if len(args) > 1 {
				fmt.Fprint(stdout, strings.ToUpper(args[1]))
			}
			return nil
		},
	}); err != nil {
		t.Fatalf("register: %v", err)
	}

	result, err := Runner{Builtins: reg}.Run(context.Background(), RunRequest{Command: "upper gode"})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.Stdout != "GODE" || result.ExitCode != 0 {
		t.Fatalf("result = %#v", result)
	}
}

func TestBuiltinStdoutParticipatesInPipeline(t *testing.T) {
	reg := NewBuiltinRegistry()
	if err := reg.Register(Builtin{
		Name: "emit_lines",
		Run: func(_ context.Context, _ []string, _ io.Reader, stdout io.Writer, _ io.Writer) error {
			fmt.Fprint(stdout, "a\nb\n")
			return nil
		},
	}); err != nil {
		t.Fatalf("register: %v", err)
	}

	result, err := Runner{Builtins: reg}.Run(context.Background(), RunRequest{Command: "emit_lines | grep b"})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.Stdout != "b\n" || result.ExitCode != 0 {
		t.Fatalf("result = %#v", result)
	}
}

func TestBuiltinStderrParticipatesInRedirection(t *testing.T) {
	dir := t.TempDir()
	reg := NewBuiltinRegistry()
	if err := reg.Register(Builtin{
		Name: "warn",
		Run: func(_ context.Context, _ []string, _ io.Reader, _ io.Writer, stderr io.Writer) error {
			fmt.Fprint(stderr, "warning")
			return nil
		},
	}); err != nil {
		t.Fatalf("register: %v", err)
	}

	result, err := Runner{Builtins: reg}.Run(context.Background(), RunRequest{Command: "warn 2>err.txt", Dir: dir})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.Stderr != "" || result.ExitCode != 0 {
		t.Fatalf("result = %#v", result)
	}
	data, err := os.ReadFile(filepath.Join(dir, "err.txt"))
	if err != nil {
		t.Fatalf("read redirected stderr: %v", err)
	}
	if string(data) != "warning" {
		t.Fatalf("stderr file = %q", data)
	}
}

func TestBuiltinNonZeroExitUsesExitStatus(t *testing.T) {
	reg := NewBuiltinRegistry()
	if err := reg.Register(Builtin{
		Name: "fail_seven",
		Run: func(context.Context, []string, io.Reader, io.Writer, io.Writer) error {
			return interp.ExitStatus(7)
		},
	}); err != nil {
		t.Fatalf("register: %v", err)
	}

	result, err := Runner{Builtins: reg}.Run(context.Background(), RunRequest{Command: "fail_seven"})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.ExitCode != 7 {
		t.Fatalf("result = %#v", result)
	}
}

func TestBuiltinCancellationReturnsContextError(t *testing.T) {
	reg := NewBuiltinRegistry()
	if err := reg.Register(Builtin{
		Name: "wait_cancel",
		Run: func(ctx context.Context, _ []string, _ io.Reader, _ io.Writer, _ io.Writer) error {
			<-ctx.Done()
			return ctx.Err()
		},
	}); err != nil {
		t.Fatalf("register: %v", err)
	}

	start := time.Now()
	result, err := Runner{Builtins: reg}.Run(context.Background(), RunRequest{Command: "wait_cancel", Timeout: 50 * time.Millisecond})
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
