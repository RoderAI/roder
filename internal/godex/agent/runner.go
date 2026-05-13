package agent

import (
	"context"
	"fmt"
	"strings"
	"sync"
	"time"

	"github.com/google/uuid"
	godecommands "github.com/pandelisz/gode/internal/godex/commands"
	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/goals"
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
	Turns                 *session.TurnStore
	Items                 *session.ItemStore
	Messages              *messagestore.Store
	Tools                 *tools.Registry
	Provider              provider.Provider
	Model                 string
	Workspace             string
	DisableAutoCompaction bool
	AutoCompactTokenLimit int
	Goals                 *goals.Runtime
	ContextMessages       []provider.Message
	Skills                []godeskills.Skill
	ActiveSkills          map[string]bool
	LoadActiveSkills      func(context.Context) (map[string]bool, error)
	Commands              []godecommands.Command
}

type Runner struct {
	bus                   *eventbus.Bus
	journal               *journal.Store
	sessions              *session.Store
	turns                 *session.TurnStore
	items                 *session.ItemStore
	messages              *messagestore.Store
	tools                 *tools.Registry
	provider              provider.Provider
	model                 string
	workspace             string
	disableAutoCompaction bool
	autoCompactTokenLimit int
	goals                 *goals.Runtime
	contextMessages       []provider.Message
	skills                []godeskills.Skill
	activeSkills          map[string]bool
	loadActiveSkills      func(context.Context) (map[string]bool, error)
	commands              []godecommands.Command
	activeMu              sync.RWMutex
	activeRuns            map[string]*activeRun
}

type RunRequest struct {
	SessionID      string
	RunID          string
	Prompt         string
	Resume         bool
	ResumeMode     session.ResumeMode
	Instructions   string
	ResponseFormat string
	Messages       []provider.Message
	InputItems     []provider.Item
	ReplacePrompt  bool
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
		turns:                 cfg.Turns,
		items:                 cfg.Items,
		messages:              cfg.Messages,
		tools:                 cfg.Tools,
		provider:              cfg.Provider,
		model:                 cfg.Model,
		workspace:             cfg.Workspace,
		disableAutoCompaction: cfg.DisableAutoCompaction,
		autoCompactTokenLimit: cfg.AutoCompactTokenLimit,
		goals:                 cfg.Goals,
		contextMessages:       append([]provider.Message(nil), cfg.ContextMessages...),
		skills:                append([]godeskills.Skill(nil), cfg.Skills...),
		activeSkills:          cloneActiveSkills(cfg.ActiveSkills),
		loadActiveSkills:      cfg.LoadActiveSkills,
		commands:              append([]godecommands.Command(nil), cfg.Commands...),
		activeRuns:            map[string]*activeRun{},
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
	active := r.registerActiveRun(req)
	defer r.unregisterActiveRun(active)
	if r.sessions != nil {
		if _, err := r.sessions.Ensure(ctx, session.Session{
			ID:        req.SessionID,
			Title:     req.Prompt,
			Workspace: r.workspace,
			Model:     r.model,
			Provider:  r.providerName(),
		}); err != nil {
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
	if err := r.persistUserItem(ctx, req); err != nil {
		return RunResult{}, r.fail(ctx, req, err)
	}
	r.emit(ctx, eventbus.Event{
		Kind:      eventbus.KindRunStarted,
		Source:    eventbus.SourceAgent,
		SessionID: req.SessionID,
		RunID:     req.RunID,
		Payload:   map[string]any{"provider": r.provider.Name()},
	})
	if err := r.startTurn(ctx, req); err != nil {
		return RunResult{}, r.fail(ctx, req, err)
	}

	commandExpansion, err := godecommands.Expand(ctx, req.Prompt, godecommands.Catalog{Commands: r.commands})
	if err != nil {
		return RunResult{}, r.fail(ctx, req, err)
	}
	activeSkills, err := r.activeSkillSettings(ctx)
	if err != nil {
		return RunResult{}, r.fail(ctx, req, err)
	}
	invocation := godeskills.ApplyInvocationsFiltered(commandExpansion.Prompt, godeskills.Catalog{Skills: r.skills}, activeSkills)
	runMessages := append([]provider.Message{}, invocation.Messages...)
	runMessages = append(runMessages, skillDiagnosticMessages(invocation.Diagnostics)...)
	contextWindow, err := r.initialContext(ctx, req, runMessages, invocation.Prompt)
	if err != nil {
		return RunResult{}, r.fail(ctx, req, err)
	}
	messages := contextWindow.Messages
	inputItems := contextWindow.InputItems
	final := ""
	stats := runStats{}
	for {
		messages, err = r.compactMessagesIfNeeded(ctx, req, messages)
		if err != nil {
			return RunResult{}, r.fail(ctx, req, err)
		}
		inputItems = providerItemsFromProviderMessages(messages)
		compaction := r.compactionOptions(ctx, req, messages)
		providerReq := provider.Request{
			SessionID:      req.SessionID,
			RunID:          req.RunID,
			PromptCacheKey: r.promptCacheKey(),
			Instructions:   firstNonEmpty(req.Instructions, GodeInstructions),
			ResponseFormat: req.ResponseFormat,
			Messages:       messages,
			InputItems:     inputItems,
			Tools:          r.providerToolSpecs(),
			Compaction:     compaction,
		}
		started := time.Now()
		outcome, err := r.streamProviderTurn(ctx, req, providerReq, messages, final, &stats, true)
		if err != nil {
			return RunResult{}, r.fail(ctx, req, err)
		}
		r.recordGoalUsage(ctx, req, outcome.Usage, time.Since(started))
		messages = outcome.Messages
		appliedSteers := false
		if steers := r.drainSteers(active); len(steers) > 0 {
			messages = r.appendSteers(ctx, req, messages, steers)
			appliedSteers = true
		}
		inputItems = providerItemsFromProviderMessages(messages)
		if outcome.HadToolCall {
			stats.ToolTurns++
			final = ""
		} else {
			final = outcome.Final
		}
		if !outcome.HadToolCall && appliedSteers {
			final = ""
			continue
		}
		if !outcome.HadToolCall {
			r.unregisterActiveRun(active)
			break
		}
	}
	if final == "" {
		return RunResult{}, r.fail(ctx, req, r.emptyCompletionError(req, stats))
	}
	if err := r.completeTurn(ctx, req, final, stats.LastResponseID); err != nil {
		return RunResult{}, r.fail(ctx, req, err)
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

type initialContext struct {
	Messages   []provider.Message
	InputItems []provider.Item
}

func (r *Runner) initialContext(ctx context.Context, req RunRequest, runMessages []provider.Message, prompt string) (initialContext, error) {
	messages := append([]provider.Message(nil), r.contextMessages...)
	messages = append(messages, runMessages...)
	messages = append(messages, req.Messages...)
	inputItems := providerItemsFromProviderMessages(messages)
	if len(req.InputItems) > 0 {
		messages = append(providerMessagesFromProviderItems(req.InputItems), messages...)
		inputItems = append(append([]provider.Item(nil), req.InputItems...), inputItems...)
	}
	if req.Resume && r.items != nil {
		priorItems, err := r.items.ListBySession(ctx, req.SessionID)
		if err != nil {
			return initialContext{}, err
		}
		priorSessionItems := excludeRunItems(priorItems, req.RunID)
		prior := providerMessagesFromSessionItems(priorSessionItems)
		if len(prior) > 0 {
			messages = append(prior, messages...)
			inputItems = append(providerItemsFromSessionItems(priorSessionItems), inputItems...)
		}
	}
	if goalMessage, ok, err := r.goalContextMessage(ctx, req.SessionID); err != nil {
		return initialContext{}, err
	} else if ok {
		messages = append(messages, goalMessage)
		inputItems = append(inputItems, providerItemFromProviderMessage(goalMessage))
	}
	if !req.ReplacePrompt {
		userMessage := provider.Message{Role: provider.RoleUser, Content: prompt}
		messages = append(messages, userMessage)
		inputItems = append(inputItems, providerItemFromProviderMessage(userMessage))
	}
	return initialContext{Messages: messages, InputItems: inputItems}, nil
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
			out = append(out, provider.Message{Role: provider.RoleAssistant, Phase: msg.Phase, Content: msg.Text})
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

func (r *Runner) emptyCompletionError(req RunRequest, stats runStats) error {
	lines := []string{
		"agent stopped without final text",
		"",
		"debug:",
		"session_id: " + req.SessionID,
		"run_id: " + req.RunID,
		fmt.Sprintf("tool_turns: %d", stats.ToolTurns),
		fmt.Sprintf("tool_calls: %d", stats.ToolCalls),
		fmt.Sprintf("tokens_used: %d", stats.TokenUsage.Total()),
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
		"reason: the provider completed a turn without final assistant text.",
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
	_ = r.failTurn(ctx, req, detail)
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
