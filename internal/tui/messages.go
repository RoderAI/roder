package tui

import (
	"github.com/pandelisz/gode/internal/godex/agent"
	"github.com/pandelisz/gode/internal/godex/eventbus"
)

type eventMsg struct {
	Event eventbus.Event
}

type runDoneMsg struct {
	Result agent.RunResult
	Err    error
}

type steerDoneMsg struct {
	RunID string
	Err   error
}

type codexAuthDoneMsg struct {
	AccountID string
	Err       error
}

type skillsInstallDoneMsg struct {
	Installed int
	Source    string
	Output    string
	Err       error
}
