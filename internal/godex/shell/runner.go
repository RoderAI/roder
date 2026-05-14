package shell

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"strings"

	"mvdan.cc/sh/v3/expand"
	"mvdan.cc/sh/v3/interp"
	"mvdan.cc/sh/v3/syntax"
)

func NewRunner() Runner {
	return Runner{}
}

func (runnerConfig Runner) Run(ctx context.Context, req RunRequest) (RunResult, error) {
	if ctx == nil {
		ctx = context.Background()
	}
	var result RunResult
	command := strings.TrimSpace(req.Command)
	if command == "" {
		return result, nil
	}

	stdout := &bytes.Buffer{}
	stderr := &bytes.Buffer{}
	parser := syntax.NewParser(syntax.Variant(syntax.LangPOSIX))
	file, err := parser.Parse(strings.NewReader(command), "gode-shell")
	if err != nil {
		result.ExitCode = 2
		fmt.Fprintf(stderr, "parse error: %v\n", err)
		result.Stderr = stderr.String()
		return result, nil
	}

	runCtx := ctx
	cancel := func() {}
	if req.Timeout > 0 {
		runCtx, cancel = context.WithTimeout(ctx, req.Timeout)
	}
	defer cancel()

	errWriter := io.Writer(stderr)
	outWriter := io.Writer(stdout)
	if req.CombineOutput {
		outWriter = stdout
		errWriter = stdout
	}
	env := req.Env
	if env == nil {
		env = os.Environ()
	}
	execHandlers := []func(next interp.ExecHandlerFunc) interp.ExecHandlerFunc{}
	if runnerConfig.Builtins != nil {
		execHandlers = append(execHandlers, builtinExecHandler(runnerConfig.Builtins))
	}
	options := []interp.RunnerOption{
		interp.StdIO(req.Stdin, outWriter, errWriter),
		interp.Dir(req.Dir),
		interp.Env(expand.ListEnviron(env...)),
		interp.ExecHandlers(execHandlers...),
	}
	if len(req.Args) > 0 {
		options = append(options, interp.Params(req.Args...))
	}
	runner, err := interp.New(options...)
	if err != nil {
		return result, err
	}
	runErr := runner.Run(runCtx, file)

	result.Stdout = stdout.String()
	if req.CombineOutput {
		result.Stderr = ""
	} else {
		result.Stderr = stderr.String()
	}
	if runErr == nil {
		return result, nil
	}
	if errors.Is(runErr, context.Canceled) || errors.Is(runErr, context.DeadlineExceeded) || runCtx.Err() != nil {
		result.ExitCode = -1
		if result.Stderr == "" {
			if runErr != nil {
				result.Stderr = runErr.Error()
			} else {
				result.Stderr = runCtx.Err().Error()
			}
		}
		return result, nil
	}
	var status interp.ExitStatus
	if errors.As(runErr, &status) {
		result.ExitCode = int(status)
		return result, nil
	}
	return result, runErr
}
