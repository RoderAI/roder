package shell

import (
	"bytes"
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestJQRawStringFromPipeline(t *testing.T) {
	reg := NewBuiltinRegistry()
	if err := RegisterJSONBuiltins(reg); err != nil {
		t.Fatalf("register: %v", err)
	}
	result, err := Runner{Builtins: reg}.Run(context.Background(), RunRequest{
		Command: `printf '{"name":"gode"}' | jq -r .name`,
		Policy:  &Policy{AllowExternal: false},
	})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.ExitCode != 0 || result.Stdout != "gode\n" || result.Stderr != "" {
		t.Fatalf("result = %#v", result)
	}
}

func TestJQReadsFileFromRunnerDirectory(t *testing.T) {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "file.json"), []byte(`{"name":"gode"}`), 0o600); err != nil {
		t.Fatalf("write json: %v", err)
	}
	reg := NewBuiltinRegistry()
	if err := RegisterJSONBuiltins(reg); err != nil {
		t.Fatalf("register: %v", err)
	}
	result, err := Runner{Builtins: reg}.Run(context.Background(), RunRequest{
		Command: "jq . file.json",
		Dir:     dir,
		Policy:  &Policy{AllowExternal: false},
	})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.ExitCode != 0 || !strings.Contains(result.Stdout, `"name"`) || !strings.Contains(result.Stdout, `"gode"`) {
		t.Fatalf("result = %#v", result)
	}
}

func TestJQInvalidFilterWritesStderr(t *testing.T) {
	reg := NewBuiltinRegistry()
	if err := RegisterJSONBuiltins(reg); err != nil {
		t.Fatalf("register: %v", err)
	}
	result, err := Runner{Builtins: reg}.Run(context.Background(), RunRequest{
		Command: `jq '['`,
		Stdin:   strings.NewReader(`{"name":"gode"}`),
		Policy:  &Policy{AllowExternal: false},
	})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.ExitCode == 0 || result.Stderr == "" {
		t.Fatalf("result = %#v", result)
	}
}

func TestJQInvalidJSONWritesStderr(t *testing.T) {
	reg := NewBuiltinRegistry()
	if err := RegisterJSONBuiltins(reg); err != nil {
		t.Fatalf("register: %v", err)
	}
	result, err := Runner{Builtins: reg}.Run(context.Background(), RunRequest{
		Command: "jq .",
		Stdin:   strings.NewReader("{"),
		Policy:  &Policy{AllowExternal: false},
	})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.ExitCode == 0 || result.Stderr == "" {
		t.Fatalf("result = %#v", result)
	}
}

func TestJQChecksContextWhileIteratingInputs(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	var stdout bytes.Buffer
	var stderr bytes.Buffer
	err := handleJQ(ctx, []string{"jq", "."}, strings.NewReader("{}\n{}"), &stdout, &stderr)
	if !errors.Is(err, context.Canceled) {
		t.Fatalf("err = %v stdout=%q stderr=%q", err, stdout.String(), stderr.String())
	}

	reg := NewBuiltinRegistry()
	if err := RegisterJSONBuiltins(reg); err != nil {
		t.Fatalf("register: %v", err)
	}
	result, err := Runner{Builtins: reg}.Run(context.Background(), RunRequest{
		Command: "jq .",
		Stdin:   strings.NewReader(strings.Repeat("{}\n", 10000)),
		Policy:  &Policy{AllowExternal: false},
		Timeout: time.Nanosecond,
	})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.ExitCode != -1 {
		t.Fatalf("result = %#v", result)
	}
}
