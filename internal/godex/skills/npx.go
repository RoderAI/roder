package skills

import (
	"bytes"
	"context"
	"os/exec"
	"strings"
)

type InstallScope string

const (
	InstallScopeGlobal  InstallScope = "global"
	InstallScopeProject InstallScope = "project"
)

type CommandRunner func(context.Context, []string) (string, string, error)

func NPXInstallCommand(source string, scope InstallScope, workspace string, dataDir string) []string {
	args := []string{"npx", "--yes", "skills", "add", source}
	if scope == InstallScopeProject {
		args = append(args, "--project")
		if strings.TrimSpace(workspace) != "" {
			args = append(args, "--cwd", workspace)
		}
		return args
	}
	args = append(args, "--global")
	return args
}

func defaultCommandRunner(ctx context.Context, command []string) (string, string, error) {
	if len(command) == 0 {
		return "", "", nil
	}
	cmd := exec.CommandContext(ctx, command[0], command[1:]...)
	var stdout bytes.Buffer
	var stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	err := cmd.Run()
	return stdout.String(), stderr.String(), err
}
