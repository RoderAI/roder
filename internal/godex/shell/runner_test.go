package shell

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestRunnerExecutesSimpleCommand(t *testing.T) {
	result, err := Runner{}.Run(context.Background(), RunRequest{
		Command: "printf hello",
	})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.Stdout != "hello" || result.Stderr != "" || result.ExitCode != 0 {
		t.Fatalf("result = %#v", result)
	}
}

func TestRunnerSupportsPOSIXVariableAssignment(t *testing.T) {
	result, err := Runner{}.Run(context.Background(), RunRequest{
		Command: `name=gode; printf "$name"`,
	})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.Stdout != "gode" || result.ExitCode != 0 {
		t.Fatalf("result = %#v", result)
	}
}

func TestRunnerSupportsPipelines(t *testing.T) {
	result, err := Runner{}.Run(context.Background(), RunRequest{
		Command: "printf 'a\\nb\\n' | grep b",
	})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.Stdout != "b\n" || result.ExitCode != 0 {
		t.Fatalf("result = %#v", result)
	}
}

func TestRunnerSupportsRedirectionInWorkspace(t *testing.T) {
	dir := t.TempDir()
	result, err := Runner{}.Run(context.Background(), RunRequest{
		Command: "printf redirected > out.txt",
		Dir:     dir,
	})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.Stdout != "" || result.Stderr != "" || result.ExitCode != 0 {
		t.Fatalf("result = %#v", result)
	}
	data, err := os.ReadFile(filepath.Join(dir, "out.txt"))
	if err != nil {
		t.Fatalf("read redirected file: %v", err)
	}
	if string(data) != "redirected" {
		t.Fatalf("redirected data = %q", data)
	}
}

func TestRunnerInvalidSyntaxReturnsExitCodeTwo(t *testing.T) {
	result, err := Runner{}.Run(context.Background(), RunRequest{
		Command: "if",
	})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.ExitCode != 2 || !strings.Contains(result.Stderr, "parse error") {
		t.Fatalf("result = %#v", result)
	}
}

func TestRunnerTimeoutReturnsExitCodeMinusOne(t *testing.T) {
	start := time.Now()
	result, err := Runner{}.Run(context.Background(), RunRequest{
		Command: "while :; do :; done",
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
