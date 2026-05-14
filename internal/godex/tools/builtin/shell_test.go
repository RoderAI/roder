package builtin

import (
	"context"
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	godexshell "github.com/pandelisz/gode/internal/godex/shell"
	"github.com/pandelisz/gode/internal/godex/tools"
	"mvdan.cc/sh/v3/interp"
)

func TestShellToolUsesEmbeddedRunner(t *testing.T) {
	root := t.TempDir()
	builtins := godexshell.NewBuiltinRegistry()
	if err := builtins.Register(godexshell.Builtin{
		Name:     "only_in_runner",
		ReadOnly: true,
		Run: func(ctx context.Context, args []string, stdin io.Reader, stdout io.Writer, stderr io.Writer) error {
			_, err := io.WriteString(stdout, "embedded\n")
			return err
		},
	}); err != nil {
		t.Fatalf("register builtin: %v", err)
	}
	reg := tools.NewRegistry()
	RegisterShell(reg, root, godexshell.Runner{Builtins: builtins})

	result, err := reg.Run(context.Background(), tools.Call{Name: "shell", Input: map[string]any{"command": "only_in_runner"}})
	if err != nil {
		t.Fatalf("run shell: %v", err)
	}
	if result.Error != "" || result.Text != "embedded" {
		t.Fatalf("shell result = %#v", result)
	}
}

func TestShellToolCombinesStdoutAndStderr(t *testing.T) {
	root := t.TempDir()
	reg := tools.NewRegistry()
	RegisterShell(reg, root, godexshell.NewRunner())

	result, err := reg.Run(context.Background(), tools.Call{Name: "shell", Input: map[string]any{"command": "printf out; printf err >&2"}})
	if err != nil {
		t.Fatalf("run shell: %v", err)
	}
	if result.Error != "" || result.Text != "outerr" {
		t.Fatalf("shell result = %#v", result)
	}
}

func TestShellToolRunsJSONAndWorkspaceBuiltins(t *testing.T) {
	root := t.TempDir()
	writeFile(t, filepath.Join(root, "file.json"), `{"name":"gode"}`+"\n")

	toolReg := tools.NewRegistry()
	RegisterFilesystem(toolReg, root)
	RegisterPatch(toolReg, root)
	builtins := godexshell.NewBuiltinRegistry()
	if err := godexshell.RegisterJSONBuiltins(builtins); err != nil {
		t.Fatalf("register json builtins: %v", err)
	}
	if err := godexshell.RegisterWorkspaceBuiltins(builtins, root, toolReg); err != nil {
		t.Fatalf("register workspace builtins: %v", err)
	}
	reg := tools.NewRegistry()
	RegisterShell(reg, root, godexshell.Runner{Builtins: builtins})

	result, err := reg.Run(context.Background(), tools.Call{Name: "shell", Input: map[string]any{
		"command": "jq -r .name file.json; gode_list_files .",
	}})
	if err != nil {
		t.Fatalf("run shell: %v", err)
	}
	if result.Error != "" || !strings.Contains(result.Text, "gode") || !strings.Contains(result.Text, "file.json") {
		t.Fatalf("shell result = %#v", result)
	}
}

func TestShellToolDefaultTimeoutIsTwoMinutes(t *testing.T) {
	root := t.TempDir()
	runner := &recordingShellRunner{}
	reg := tools.NewRegistry()
	RegisterShell(reg, root, runner)

	if _, err := reg.Run(context.Background(), tools.Call{Name: "shell", Input: map[string]any{"command": "printf hi"}}); err != nil {
		t.Fatalf("run shell: %v", err)
	}
	if len(runner.requests) != 1 {
		t.Fatalf("requests = %d", len(runner.requests))
	}
	if runner.requests[0].Timeout != 2*time.Minute {
		t.Fatalf("timeout = %s", runner.requests[0].Timeout)
	}
	if runner.requests[0].CombineOutput != true {
		t.Fatalf("combine output = false")
	}
}

func TestShellToolRequiresPermissionWhenAutoApproveDisabled(t *testing.T) {
	root := t.TempDir()
	runner := &recordingShellRunner{}
	reg := tools.NewRegistry(tools.WithAutoApprove(false))
	RegisterShell(reg, root, runner)

	result, err := reg.Run(context.Background(), tools.Call{Name: "shell", Input: map[string]any{"command": "printf hi"}})
	if err != nil {
		t.Fatalf("run should return permission denial as tool result: %v", err)
	}
	if result.Error == "" || !strings.Contains(result.Text, "permission required") {
		t.Fatalf("permission result = %#v", result)
	}
	if len(runner.requests) != 0 {
		t.Fatalf("runner should not be called before permission, requests = %d", len(runner.requests))
	}
}

func TestShellToolReturnsNonZeroExitAsFailedResult(t *testing.T) {
	root := t.TempDir()
	reg := tools.NewRegistry()
	RegisterShell(reg, root, godexshell.Runner{Builtins: mustBuiltinRegistry(t, godexshell.Builtin{
		Name: "fail_three",
		Run: func(context.Context, []string, io.Reader, io.Writer, io.Writer) error {
			return interp.ExitStatus(3)
		},
	})})

	result, err := reg.Run(context.Background(), tools.Call{Name: "shell", Input: map[string]any{"command": "fail_three"}})
	if err != nil {
		t.Fatalf("run should return shell failure as tool result: %v", err)
	}
	if result.Error == "" || !strings.Contains(result.Text, "shell exited with status 3") {
		t.Fatalf("failure result = %#v", result)
	}
}

type recordingShellRunner struct {
	requests []godexshell.RunRequest
	result   godexshell.RunResult
	err      error
}

func (r *recordingShellRunner) Run(ctx context.Context, req godexshell.RunRequest) (godexshell.RunResult, error) {
	r.requests = append(r.requests, req)
	return r.result, r.err
}

func mustBuiltinRegistry(t *testing.T, builtins ...godexshell.Builtin) *godexshell.BuiltinRegistry {
	t.Helper()
	reg := godexshell.NewBuiltinRegistry()
	for _, builtin := range builtins {
		if err := reg.Register(builtin); err != nil {
			t.Fatalf("register builtin: %v", err)
		}
	}
	return reg
}

func writeFile(t *testing.T, path string, content string) {
	t.Helper()
	if err := os.WriteFile(path, []byte(content), 0o600); err != nil {
		t.Fatalf("write %s: %v", path, err)
	}
}
