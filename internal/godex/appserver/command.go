package appserver

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"io"
	"os"
	"os/exec"
	"sync"
	"time"
)

type activeCommand struct {
	cancel context.CancelFunc
	stdin  io.WriteCloser
}

type commandExecParams struct {
	Command            []string           `json:"command"`
	ProcessID          string             `json:"processId"`
	CWD                string             `json:"cwd"`
	Env                map[string]*string `json:"env"`
	StreamStdin        bool               `json:"streamStdin"`
	StreamStdoutStderr bool               `json:"streamStdoutStderr"`
	TimeoutMS          *int64             `json:"timeoutMs"`
	DisableTimeout     bool               `json:"disableTimeout"`
	DisableOutputCap   bool               `json:"disableOutputCap"`
	OutputBytesCap     *int64             `json:"outputBytesCap"`
	TTY                bool               `json:"tty"`
}

func (s *Server) handleCommandExec(ctx context.Context, conn *Connection, raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[commandExecParams](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	if len(params.Command) == 0 {
		return nil, rpcError(errorInvalidParams, "command must not be empty")
	}
	if params.TTY {
		return nil, rpcError(errorInvalidParams, "tty command execution is not supported yet")
	}
	if params.StreamStdin && params.ProcessID == "" {
		return nil, rpcError(errorInvalidParams, "streamStdin requires processId")
	}
	if params.StreamStdoutStderr && params.ProcessID == "" {
		return nil, rpcError(errorInvalidParams, "streamStdoutStderr requires processId")
	}
	if params.DisableTimeout && params.TimeoutMS != nil {
		return nil, rpcError(errorInvalidParams, "cannot set both timeoutMs and disableTimeout")
	}

	runCtx := ctx
	cancel := func() {}
	if !params.DisableTimeout {
		timeout := 30 * time.Second
		if params.TimeoutMS != nil {
			if *params.TimeoutMS < 0 {
				return nil, rpcError(errorInvalidParams, "timeoutMs must be non-negative")
			}
			timeout = time.Duration(*params.TimeoutMS) * time.Millisecond
		}
		runCtx, cancel = context.WithTimeout(ctx, timeout)
	} else {
		runCtx, cancel = context.WithCancel(ctx)
	}
	defer cancel()

	cmd := exec.CommandContext(runCtx, params.Command[0], params.Command[1:]...)
	if params.CWD != "" {
		if err := requireAbsolutePath(params.CWD); err != nil {
			return nil, rpcError(errorInvalidParams, err.Error())
		}
		cmd.Dir = params.CWD
	} else {
		cmd.Dir = s.app.Config.Workspace
	}
	cmd.Env = mergedEnv(params.Env)

	stdoutPipe, err := cmd.StdoutPipe()
	if err != nil {
		return nil, rpcError(errorInternal, err.Error())
	}
	stderrPipe, err := cmd.StderrPipe()
	if err != nil {
		return nil, rpcError(errorInternal, err.Error())
	}
	stdinPipe, err := cmd.StdinPipe()
	if err != nil {
		return nil, rpcError(errorInternal, err.Error())
	}

	if err := cmd.Start(); err != nil {
		return nil, rpcError(errorInternal, err.Error())
	}
	if params.ProcessID != "" {
		s.mu.Lock()
		s.commands[params.ProcessID] = &activeCommand{cancel: cancel, stdin: stdinPipe}
		s.mu.Unlock()
		defer func() {
			s.mu.Lock()
			delete(s.commands, params.ProcessID)
			s.mu.Unlock()
		}()
	}

	var stdout bytes.Buffer
	var stderr bytes.Buffer
	var wg sync.WaitGroup
	wg.Add(2)
	go func() {
		defer wg.Done()
		copyCommandOutput(ctx, conn, params, "stdout", stdoutPipe, &stdout)
	}()
	go func() {
		defer wg.Done()
		copyCommandOutput(ctx, conn, params, "stderr", stderrPipe, &stderr)
	}()
	wg.Wait()
	waitErr := cmd.Wait()

	exitCode := 0
	if waitErr != nil {
		if exitErr, ok := waitErr.(*exec.ExitError); ok {
			exitCode = exitErr.ExitCode()
		} else if runCtx.Err() != nil {
			exitCode = -1
		} else {
			return nil, rpcError(errorInternal, waitErr.Error())
		}
	}
	if runCtx.Err() == context.DeadlineExceeded {
		exitCode = -1
	}

	return map[string]any{
		"exitCode": exitCode,
		"stdout":   stdout.String(),
		"stderr":   stderr.String(),
	}, nil
}

func (s *Server) handleCommandWrite(raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[struct {
		ProcessID   string `json:"processId"`
		DeltaBase64 string `json:"deltaBase64"`
		CloseStdin  bool   `json:"closeStdin"`
	}](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	s.mu.RLock()
	command := s.commands[params.ProcessID]
	s.mu.RUnlock()
	if command == nil {
		return nil, rpcError(errorInvalidParams, "process not found")
	}
	if params.DeltaBase64 != "" {
		data, err := base64.StdEncoding.DecodeString(params.DeltaBase64)
		if err != nil {
			return nil, rpcError(errorInvalidParams, "deltaBase64 is not valid base64")
		}
		if _, err := command.stdin.Write(data); err != nil {
			return nil, rpcError(errorInternal, err.Error())
		}
	}
	if params.CloseStdin {
		if err := command.stdin.Close(); err != nil {
			return nil, rpcError(errorInternal, err.Error())
		}
	}
	return map[string]any{}, nil
}

func (s *Server) handleCommandTerminate(raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[struct {
		ProcessID string `json:"processId"`
	}](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	s.mu.RLock()
	command := s.commands[params.ProcessID]
	s.mu.RUnlock()
	if command == nil {
		return nil, rpcError(errorInvalidParams, "process not found")
	}
	command.cancel()
	return map[string]any{}, nil
}

func copyCommandOutput(ctx context.Context, conn *Connection, params commandExecParams, stream string, reader io.Reader, buffer *bytes.Buffer) {
	tmp := make([]byte, 8192)
	for {
		n, err := reader.Read(tmp)
		if n > 0 {
			chunk := tmp[:n]
			if !params.StreamStdoutStderr {
				_, _ = buffer.Write(chunk)
			}
			if params.StreamStdoutStderr {
				_ = conn.sendNotification(ctx, "command/exec/outputDelta", map[string]any{
					"processId":   params.ProcessID,
					"stream":      stream,
					"deltaBase64": base64.StdEncoding.EncodeToString(chunk),
					"capReached":  false,
				})
			}
		}
		if err != nil {
			return
		}
	}
}

func mergedEnv(overrides map[string]*string) []string {
	env := os.Environ()
	if len(overrides) == 0 {
		return env
	}
	values := make(map[string]string, len(env)+len(overrides))
	for _, entry := range env {
		key, value, ok := cutEnv(entry)
		if ok {
			values[key] = value
		}
	}
	for key, value := range overrides {
		if value == nil {
			delete(values, key)
		} else {
			values[key] = *value
		}
	}
	out := make([]string, 0, len(values))
	for key, value := range values {
		out = append(out, key+"="+value)
	}
	return out
}

func cutEnv(entry string) (string, string, bool) {
	for i, r := range entry {
		if r == '=' {
			return entry[:i], entry[i+1:], true
		}
	}
	return "", "", false
}
