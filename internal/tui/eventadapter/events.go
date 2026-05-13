package eventadapter

import (
	"strings"

	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

const (
	KindHookResult    eventbus.Kind = "hook.result"
	KindSessionUpdate eventbus.Kind = "session.updated"
	KindModelChanged  eventbus.Kind = "model.changed"
)

type Message struct {
	Key   string
	Role  viewmodel.Role
	Title string
	Body  string
}

type Update struct {
	Messages            []Message
	AssistantDelta      string
	ReasoningDelta      string
	ReasoningSummary    string
	HasReasoningSummary bool
	ContextUsedPercent  float64
	HasContextTokens    bool
	Status              string
	HasStatus           bool
	Running             *bool
}

func Apply(ev eventbus.Event) Update {
	switch ev.Kind {
	case eventbus.KindAssistantDelta:
		var payload textPayload
		_ = ev.DecodePayload(&payload)
		return Update{AssistantDelta: payload.Text}
	case eventbus.KindReasoningSummaryDelta:
		var payload textPayload
		_ = ev.DecodePayload(&payload)
		return withStatus(Update{ReasoningDelta: payload.Text}, "reasoning")
	case eventbus.KindReasoningSummaryCompleted:
		var payload textPayload
		_ = ev.DecodePayload(&payload)
		return Update{ReasoningSummary: payload.Text, HasReasoningSummary: payload.Text != ""}
	case eventbus.KindContextTokensUpdated:
		var payload contextTokensPayload
		_ = ev.DecodePayload(&payload)
		return Update{ContextUsedPercent: payload.Percent, HasContextTokens: true}
	case eventbus.KindUserSteerSubmitted:
		return status("steer queued for active run")
	case eventbus.KindUserSteerApplied:
		return status("steer applied")
	case eventbus.KindAssistantCompleted:
		return status("assistant completed")
	case eventbus.KindToolRequested:
		var payload toolPayload
		_ = ev.DecodePayload(&payload)
		return withStatus(withMessageKey(payload.MessageKey(), viewmodel.RoleTool, payload.Tool, "requested"), statusWithName("tool requested", payload.Tool))
	case eventbus.KindToolStarted:
		var payload toolPayload
		_ = ev.DecodePayload(&payload)
		return status(statusWithName("tool running", payload.Tool))
	case eventbus.KindToolCompleted:
		var payload toolPayload
		_ = ev.DecodePayload(&payload)
		body := summarizeToolTimeline(payload.Tool, payload.Input, payload.Text)
		return withStatus(withMessageKey(payload.MessageKey(), viewmodel.RoleTool, payload.Tool, appendToolMetadata(body, payload)), statusWithName("tool completed", payload.Tool))
	case eventbus.KindToolFailed:
		var payload toolPayload
		_ = ev.DecodePayload(&payload)
		return withStatus(withMessageKey(payload.MessageKey(), viewmodel.RoleError, payload.Tool, payload.Error), statusWithName("tool failed", payload.Tool)+" - ctrl+l errors")
	case eventbus.KindPermissionRequested:
		var payload toolPayload
		_ = ev.DecodePayload(&payload)
		return withStatus(withMessage(viewmodel.RoleTool, payload.Tool, permissionSummary(payload)), "permission requested")
	case eventbus.KindPermissionResponded:
		var payload permissionPayload
		_ = ev.DecodePayload(&payload)
		if payload.Decision == "" {
			return status("permission responded")
		}
		return status("permission " + payload.Decision)
	case eventbus.KindMCPStateChanged:
		var payload statePayload
		_ = ev.DecodePayload(&payload)
		return status(stateStatus("mcp", payload))
	case eventbus.KindLSPStateChanged:
		var payload statePayload
		_ = ev.DecodePayload(&payload)
		return status(stateStatus("lsp", payload))
	case KindHookResult:
		var payload hookPayload
		_ = ev.DecodePayload(&payload)
		return status(hookStatus(payload))
	case KindSessionUpdate:
		var payload namedPayload
		_ = ev.DecodePayload(&payload)
		return status(statusWithName("session updated", firstNonEmpty(payload.Title, payload.ID)))
	case KindModelChanged:
		var payload namedPayload
		_ = ev.DecodePayload(&payload)
		return status(statusWithName("model changed", firstNonEmpty(payload.Model, payload.Name, payload.ID)))
	case eventbus.KindRunCompleted:
		done := false
		return withStatus(Update{Running: &done}, "run completed")
	case eventbus.KindRunFailed:
		var payload errorPayload
		_ = ev.DecodePayload(&payload)
		done := false
		return withStatus(withMessage(viewmodel.RoleError, "", payload.Message()), "run failed - ctrl+l errors").withRunning(&done)
	default:
		return Update{}
	}
}

type textPayload struct {
	Text string `json:"text"`
}

type toolPayload struct {
	ToolCallID   string         `json:"tool_call_id"`
	ToolCallID2  string         `json:"toolCallId"`
	Tool         string         `json:"tool"`
	Action       string         `json:"action"`
	Path         string         `json:"path"`
	Input        map[string]any `json:"input"`
	Text         string         `json:"text"`
	Error        string         `json:"error"`
	HookDecision string         `json:"hook_decision"`
	HookWarnings []string       `json:"hook_warnings"`
}

type permissionPayload struct {
	Decision string `json:"decision"`
}

type statePayload struct {
	Server string `json:"server"`
	State  string `json:"state"`
	Error  string `json:"error"`
}

type hookPayload struct {
	Hook     string `json:"hook"`
	Name     string `json:"name"`
	Decision string `json:"decision"`
	Error    string `json:"error"`
}

type namedPayload struct {
	ID    string `json:"id"`
	Name  string `json:"name"`
	Title string `json:"title"`
	Model string `json:"model"`
}

type errorPayload struct {
	Error  string `json:"error"`
	Detail string `json:"detail"`
}

type contextTokensPayload struct {
	Percent float64 `json:"percent"`
}

func (p toolPayload) MessageKey() string {
	id := firstNonEmpty(p.ToolCallID, p.ToolCallID2)
	if id == "" {
		return ""
	}
	return "tool:" + id
}

func (p errorPayload) Message() string {
	if strings.TrimSpace(p.Detail) != "" {
		return p.Detail
	}
	return p.Error
}

func withMessage(role viewmodel.Role, title string, body string) Update {
	return withMessageKey("", role, title, body)
}

func withMessageKey(key string, role viewmodel.Role, title string, body string) Update {
	title = strings.TrimSpace(title)
	body = strings.TrimSpace(body)
	if title == "" && body == "" {
		return Update{}
	}
	return Update{Messages: []Message{{Key: key, Role: role, Title: title, Body: body}}}
}

func status(text string) Update {
	return withStatus(Update{}, text)
}

func withStatus(update Update, text string) Update {
	text = strings.TrimSpace(text)
	if text == "" {
		return update
	}
	update.Status = text
	update.HasStatus = true
	return update
}

func (u Update) withRunning(running *bool) Update {
	u.Running = running
	return u
}

func statusWithName(prefix string, name string) string {
	name = strings.TrimSpace(name)
	if name == "" {
		return prefix
	}
	return prefix + ": " + name
}

func stateStatus(prefix string, payload statePayload) string {
	base := statusWithName(prefix, payload.Server)
	if payload.State != "" {
		base += " " + payload.State
	}
	if payload.Error != "" {
		base += " - " + payload.Error
	}
	return base
}

func hookStatus(payload hookPayload) string {
	name := firstNonEmpty(payload.Hook, payload.Name)
	base := statusWithName("hook result", name)
	if payload.Decision != "" {
		base += ": " + payload.Decision
	}
	if payload.Error != "" {
		base += " - " + payload.Error
	}
	return base
}

func firstNonEmpty(values ...string) string {
	for _, value := range values {
		if trimmed := strings.TrimSpace(value); trimmed != "" {
			return trimmed
		}
	}
	return ""
}

func summarizeToolTimeline(tool string, input map[string]any, output string) string {
	switch tool {
	case "read_file":
		path := strings.TrimSpace(inputString(input, "path"))
		if path == "" {
			return "read file"
		}
		return "read " + path
	default:
		return truncate(output, 1600)
	}
}

func appendToolMetadata(summary string, payload toolPayload) string {
	lines := []string{strings.TrimSpace(summary)}
	if payload.HookDecision != "" {
		lines = append(lines, "hook: "+payload.HookDecision)
	}
	if len(payload.HookWarnings) > 0 {
		lines = append(lines, "hook warnings: "+strings.Join(payload.HookWarnings, "; "))
	}
	return strings.TrimSpace(strings.Join(nonEmpty(lines), "\n"))
}

func permissionSummary(payload toolPayload) string {
	lines := []string{"permission requested"}
	if payload.Action != "" {
		lines = append(lines, "action: "+payload.Action)
	}
	if payload.Path != "" {
		lines = append(lines, "path: "+payload.Path)
	}
	return strings.Join(lines, "\n")
}

func nonEmpty(lines []string) []string {
	out := lines[:0]
	for _, line := range lines {
		if strings.TrimSpace(line) != "" {
			out = append(out, line)
		}
	}
	return out
}

func inputString(input map[string]any, key string) string {
	if input == nil {
		return ""
	}
	switch value := input[key].(type) {
	case string:
		return value
	case []byte:
		return string(value)
	default:
		return ""
	}
}

func truncate(text string, limit int) string {
	if len(text) <= limit {
		return text
	}
	return text[:limit] + "\n... truncated in TUI; full result is in the event journal"
}
