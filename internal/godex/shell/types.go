package shell

import (
	"io"
	"time"
)

type RunRequest struct {
	Command       string
	Args          []string
	Dir           string
	Env           []string
	Stdin         io.Reader
	Timeout       time.Duration
	Policy        *Policy
	CombineOutput bool
}

type RunResult struct {
	Stdout   string
	Stderr   string
	ExitCode int
}

type Policy struct {
	AllowExternal bool
	Blocked       map[string]string
}

type Runner struct {
	Builtins *BuiltinRegistry
}
