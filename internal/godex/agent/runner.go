package agent

import (
	"context"
	"fmt"
	"strings"
	"sync"

	"github.com/google/uuid"
	godecommands "github.com/pandelisz/gode/internal/godex/commands"
	"github.com/pandelisz/gode/internal/godex/contextwindow"
	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/journal"
	messagestore "github.com/pandelisz/gode/internal/godex/message"
	"github.com/pandelisz/gode/internal/godex/provider"
	"github.com/pandelisz/gode/internal/godex/session"
	godeskills "github.com/pandelisz/gode/internal/godex/skills"
	"github.com/pandelisz/gode/internal/godex/tools"
)

type Config struct {
	Bus                   *eventbus.Bus
	Journal               *journal.Store
	Sessions              *session.Store
	Messages              *messagestore.Store
	Tools                 *tools.Registry
	Provider              provider.Provider
	Model                 string
	DisableAutoCompaction bool
	AutoCompactTokenLimit int
	ContextMessages       []provider.Message
	Skills                []godeskills.Skill
	Commands              []godecommands.Command
}

type Runner struct {
	bus                   *eventbus.Bus
	journal               *journal.Store
	sessions              *session.Store
	messages              *messagestore.Store
	tools                 *tools.Registry
	provider              provider.Provider
	model                 string
	disableAutoCompaction bool
	autoCompactTokenLimit int
	contextMessages       []provider.Message
	skills                []godeskills.Skill
	commands              []godecommands.Command
}

type RunRequest struct {
	SessionID      string
	RunID          string
	Prompt         string
	Resume         bool
	Instructions   string
	ResponseFormat string
	Messages       []provider.Message
	MaxTurns       int
}

type RunResult struct {
	SessionID string
	RunID     string
	FinalText string
}

func NewRunner(cfg Config) *Runner {
	return &Runner{
		bus:                   cfg.Bus,
		journal:               cfg.Journal,
		sessions:              cfg.Sessions,
		messages:              cfg.Messages,
		tools:                 cfg.Tools,
		provider:              cfg.Provider,
		model:                 cfg.Model,
		disableAutoCompaction: cfg.DisableAutoCompaction,
		autoCompactTokenLimit: cfg.AutoCompactTokenLimit,
		contextMessages:       append([]provider.Message(nil), cfg.ContextMessages...),
		skills:                append([]godeskills.Skill(nil), cfg.Skills...),
		commands:              append([]godecommands.Command(nil), cfg.Commands...),
	}
}

func (r *Runner) Run(ctx context.Context, req RunRequest) (RunResult, error) {
	if req.SessionID == "" {
		req.SessionID = uuid.NewString()
	}
	if req.RunID == "" {
		req.RunID = uuid.NewString()
	}
	if r.bus == nil {
		r.bus = eventbus.New()
	}
	if r.provider == nil {
		return RunResult{}, fmt.Errorf("provider is required")
	}
	if r.sessions != nil {
		if _, err := r.sessions.Ensure(ctx, session.Session{ID: req.SessionID, Title: req.Prompt}); err != nil {
			return RunResult{}, r.fail(ctx, req, err)
		}
	}
	var eventWG sync.WaitGroup
	if r.journal != nil || r.messages != nil {
		eventCtx, cancelEvents := context.WithCancel(context.Background())
		defer func() {
			cancelEvents()
			eventWG.Wait()
			r.refreshSessionMessageCount(context.Background(), req.SessionID)
		}()
		ch := r.bus.Subscribe(eventCtx, eventbus.Filter{SessionID: req.SessionID, RunID: req.RunID})
		eventWG.Add(1)
		go func() {
			defer eventWG.Done()
			for ev := range ch {
				if r.journal != nil {
					_ = r.journal.Append(context.Background(), ev)
				}
				if r.messages != nil {
					_, _ = r.messages.AppendProjected(context.Background(), ev)
				}
			}
		}()
	}

	r.emit(ctx, eventbus.Event{
		Kind:      eventbus.KindUserPromptSubmitted,
		Source:    eventbus.SourceTUI,
		SessionID: req.SessionID,
		RunID:     req.RunID,
		Payload:   map[string]any{"prompt": req.Prompt},
	})
	r.emit(ctx, eventbus.Event{
		Kind:      eventbus.KindRunStarted,
		Source:    eventbus.SourceAgent,
		SessionID: req.SessionID,
		RunID:     req.RunID,
		Payload:   map[string]any{"provider": r.provider.Name()},
	})

	commandExpansion, err := godecommands.Expand(ctx, req.Prompt, godecommands.Catalog{Commands: r.commands})
	if err != nil {
		return RunResult{}, r.fail(ctx, req, err)
	}
	invocation := godeskills.ApplyInvocations(commandExpansion.Prompt, godeskills.Catalog{Skills: r.skills})
	messages, err := r.initialMessages(ctx, req, invocation.Messages, invocation.Prompt)
	if err != nil {
		return RunResult{}, r.fail(ctx, req, err)
	}
	maxTurns := req.MaxTurns
	if maxTurns <= 0 {
		maxTurns = 32
	}

	final := ""
	stats := runStats{}
	for turn := 0; turn < maxTurns; turn++ {
		compaction := r.compactionOptions(ctx, req, messages)
		providerReq := provider.Request{
			SessionID:      req.SessionID,
			RunID:          req.RunID,
			Instructions:   firstNonEmpty(req.Instructions, GodeInstructions),
			ResponseFormat: req.ResponseFormat,
			Messages:       messages,
			Tools:          r.providerToolSpecs(),
			Compaction:     compaction,
		}
		outcome, err := r.streamProviderTurn(ctx, req, providerReq, messages, final, &stats, true)
		if err != nil {
			return RunResult{}, r.fail(ctx, req, err)
		}
		messages = outcome.Messages
		final = outcome.Final
		if outcome.HadToolCall {
			stats.ToolTurns++
		}
		if !outcome.HadToolCall || outcome.ProducedText {
			break
		}
	}
	if final == "" && stats.ToolCalls > 0 {
		messages = append(messages, provider.Message{Role: provider.RoleUser, Content: finalAfterToolBudgetPrompt(maxTurns)})
		compaction := r.compactionOptions(ctx, req, messages)
		providerReq := provider.Request{
			SessionID:      req.SessionID,
			RunID:          req.RunID,
			Instructions:   firstNonEmpty(req.Instructions, GodeInstructions),
			ResponseFormat: req.ResponseFormat,
			Messages:       messages,
			Compaction:     compaction,
		}
		outcome, err := r.streamProviderTurn(ctx, req, providerReq, messages, final, &stats, false)
		if err != nil {
			return RunResult{}, r.fail(ctx, req, err)
		}
		messages = outcome.Messages
		final = outcome.Final
	}
	if final == "" {
		return RunResult{}, r.fail(ctx, req, r.toolLoopError(req, maxTurns, stats))
	}

	r.emit(ctx, eventbus.Event{
		Kind:      eventbus.KindRunCompleted,
		Source:    eventbus.SourceAgent,
		SessionID: req.SessionID,
		RunID:     req.RunID,
		Payload:   map[string]any{"final_text": final},
	})
	return RunResult{SessionID: req.SessionID, RunID: req.RunID, FinalText: final}, nil
}

func (r *Runner) initialMessages(ctx context.Context, req RunRequest, runMessages []provider.Message, prompt string) ([]provider.Message, error) {
	messages := append([]provider.Message(nil), r.contextMessages...)
	messages = append(messages, runMessages...)
	messages = append(messages, req.Messages...)
	if req.Resume && r.messages != nil {
		prior, err := r.messages.ListBySession(ctx, req.SessionID)
		if err != nil {
			return nil, err
		}
		messages = append(providerMessages(excludeRunMessages(prior, req.RunID)), messages...)
	}
	messages = append(messages, provider.Message{Role: provider.RoleUser, Content: prompt})
	return messages, nil
}

func firstNonEmpty(values ...string) string {
	for _, value := range values {
		if strings.TrimSpace(value) != "" {
			return value
		}
	}
	return ""
}

func providerMessages(messages []messagestore.Message) []provider.Message {
	messages = canonicalProviderWindow(messages)
	out := make([]provider.Message, 0, len(messages))
	for _, msg := range messages {
		if len(msg.RawJSON) > 0 {
			out = append(out, provider.Message{RawJSON: append([]byte(nil), msg.RawJSON...)})
			continue
		}
		switch msg.Role {
		case messagestore.RoleUser:
			out = append(out, provider.Message{Role: provider.RoleUser, Content: msg.Text})
		case messagestore.RoleAssistant:
			out = append(out, provider.Message{Role: provider.RoleAssistant, Content: msg.Text})
		case messagestore.RoleTool:
			out = append(out, provider.Message{Role: provider.RoleTool, Content: msg.Text, ToolCallID: msg.ToolCallID})
		case messagestore.RoleCompaction:
			out = append(out, provider.Message{RawJSON: append([]byte(nil), msg.RawJSON...)})
		}
	}
	return out
}

func canonicalProviderWindow(messages []messagestore.Message) []messagestore.Message {
	latestCompaction := -1
	latestRunID := ""
	for i, msg := range messages {
		if msg.Role == messagestore.RoleCompaction && len(msg.RawJSON) > 0 {
			latestCompaction = i
			latestRunID = msg.RunID
		}
	}
	if latestCompaction == -1 {
		return messages
	}
	start := latestCompaction
	for i := latestCompaction; i >= 0; i-- {
		msg := messages[i]
		if msg.Role != messagestore.RoleCompaction || len(msg.RawJSON) == 0 {
			break
		}
		if latestRunID != "" && msg.RunID != latestRunID {
			break
		}
		start = i
	}
	return messages[start:]
}

func excludeRunMessages(messages []messagestore.Message, runID string) []messagestore.Message {
	if runID == "" {
		return messages
	}
	out := messages[:0]
	for _, msg := range messages {
		if msg.RunID != runID {
			out = append(out, msg)
		}
	}
	return out
}

func (r *Runner) refreshSessionMessageCount(ctx context.Context, sessionID string) {
	if r.sessions == nil || r.messages == nil || sessionID == "" {
		return
	}
	messages, err := r.messages.ListBySession(ctx, sessionID)
	if err != nil {
		return
	}
	_, _ = r.sessions.UpdateMessageCount(ctx, sessionID, len(messages))
}

func (r *Runner) compactionOptions(ctx context.Context, req RunRequest, messages []provider.Message) provider.CompactionOptions {
	model := firstNonEmpty(r.model, "gpt-5.5")
	window := contextwindow.ForModel(model)
	estimate := contextwindow.EstimateMessages(contextWindowMessages(messages), window)
	r.emit(ctx, eventbus.Event{
		Kind:      eventbus.KindContextTokensUpdated,
		Source:    eventbus.SourceAgent,
		SessionID: req.SessionID,
		RunID:     req.RunID,
		Payload: map[string]any{
			"model":          model,
			"tokens":         estimate.Tokens,
			"context_window": estimate.ContextWindow,
			"percent":        estimate.Percent,
		},
	})

	options := contextwindow.OptionsForModel(model, r.disableAutoCompaction, r.autoCompactTokenLimit)
	if !options.Enabled || r.providerName() != "openai" {
		return provider.CompactionOptions{
			Model:            options.Model,
			ContextWindow:    options.ContextWindow,
			CompactThreshold: options.CompactThreshold,
		}
	}
	r.emit(ctx, eventbus.Event{
		Kind:      eventbus.KindContextCompactionConfigured,
		Source:    eventbus.SourceAgent,
		SessionID: req.SessionID,
		RunID:     req.RunID,
		Payload: map[string]any{
			"model":             model,
			"tokens":            estimate.Tokens,
			"context_window":    options.ContextWindow,
			"compact_threshold": options.CompactThreshold,
		},
	})
	return provider.CompactionOptions{
		Enabled:          true,
		Model:            options.Model,
		ContextWindow:    options.ContextWindow,
		CompactThreshold: options.CompactThreshold,
	}
}

func contextWindowMessages(messages []provider.Message) []contextwindow.Message {
	out := make([]contextwindow.Message, 0, len(messages))
	for _, msg := range messages {
		out = append(out, contextwindow.Message{
			Role:          string(msg.Role),
			Content:       msg.Content,
			ToolCallID:    msg.ToolCallID,
			ToolName:      msg.ToolName,
			ToolArguments: msg.ToolArguments,
			RawJSON:       msg.RawJSON,
		})
	}
	return out
}

func toolResponseContent(name string, result tools.Result) string {
	text := strings.TrimSpace(result.Text)
	if text == "" && result.Error != "" {
		text = strings.TrimSpace(result.Error)
	}
	if text == "" {
		text = "(no output)"
	}
	if result.Error != "" {
		return fmt.Sprintf("Tool %s failed:\n%s", name, text)
	}
	return fmt.Sprintf("Tool %s result:\n%s", name, text)
}

func (r *Runner) providerToolSpecs() []provider.ToolSpec {
	if r.tools == nil {
		return nil
	}
	specs := r.tools.Specs()
	out := make([]provider.ToolSpec, 0, len(specs))
	for _, spec := range specs {
		out = append(out, provider.ToolSpec{
			Name:        spec.Name,
			Description: spec.Description,
			Schema:      spec.Schema,
		})
	}
	return out
}

func finalAfterToolBudgetPrompt(maxTurns int) string {
	return fmt.Sprintf("The agent hit the %d-turn safety limit while handling tool calls. Do not request more tools. Using only the tool results and context already available, provide the best final answer now. If the task is incomplete, summarize what you found and what remains.", maxTurns)
}

func (r *Runner) toolLoopError(req RunRequest, maxTurns int, stats runStats) error {
	lines := []string{
		"agent stopped without final text after tool loop",
		"",
		"debug:",
		"session_id: " + req.SessionID,
		"run_id: " + req.RunID,
		fmt.Sprintf("max_turns: %d", maxTurns),
		fmt.Sprintf("tool_turns: %d", stats.ToolTurns),
		fmt.Sprintf("tool_calls: %d", stats.ToolCalls),
		"provider: " + r.providerName(),
	}
	if stats.LastTool != "" {
		lines = append(lines, "last_tool: "+stats.LastTool)
	}
	if stats.LastToolCallID != "" {
		lines = append(lines, "last_tool_call_id: "+stats.LastToolCallID)
	}
	if r.journal != nil && r.journal.Path() != "" {
		lines = append(lines, "event_journal: "+r.journal.Path())
	}
	if r.messages != nil {
		if path := r.messages.SessionPath(req.SessionID); path != "" {
			lines = append(lines, "message_log: "+path)
		}
	}
	lines = append(lines,
		"",
		"reason: the model kept requesting tools until the safety turn limit was reached, then returned an empty assistant completion.",
		"next: inspect the event journal for this run or retry with a narrower prompt.",
	)
	return fmt.Errorf("%s", strings.Join(lines, "\n"))
}

func (r *Runner) providerName() string {
	if r.provider == nil {
		return ""
	}
	return r.provider.Name()
}

func (r *Runner) fail(ctx context.Context, req RunRequest, err error) error {
	detail := strings.TrimSpace(err.Error())
	summary := firstLine(detail)
	r.emit(ctx, eventbus.Event{
		Kind:      eventbus.KindRunFailed,
		Source:    eventbus.SourceAgent,
		SessionID: req.SessionID,
		RunID:     req.RunID,
		Payload:   map[string]any{"error": summary, "detail": detail},
	})
	return err
}

func firstLine(text string) string {
	for _, line := range strings.Split(text, "\n") {
		line = strings.TrimSpace(line)
		if line != "" {
			return line
		}
	}
	return text
}

func (r *Runner) emit(ctx context.Context, ev eventbus.Event) eventbus.Event {
	if r.bus != nil {
		ev = r.bus.Publish(ctx, ev)
	}
	return ev
}
