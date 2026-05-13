package agent

import (
	"context"
	"fmt"
	"strings"
	"sync"

	"github.com/google/uuid"
	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/journal"
	messagestore "github.com/pandelisz/gode/internal/godex/message"
	"github.com/pandelisz/gode/internal/godex/provider"
	"github.com/pandelisz/gode/internal/godex/session"
	"github.com/pandelisz/gode/internal/godex/tools"
)

type Config struct {
	Bus      *eventbus.Bus
	Journal  *journal.Store
	Sessions *session.Store
	Messages *messagestore.Store
	Tools    *tools.Registry
	Provider provider.Provider
}

type Runner struct {
	bus      *eventbus.Bus
	journal  *journal.Store
	sessions *session.Store
	messages *messagestore.Store
	tools    *tools.Registry
	provider provider.Provider
}

type RunRequest struct {
	SessionID string
	RunID     string
	Prompt    string
	Resume    bool
	Messages  []provider.Message
	MaxTurns  int
}

type RunResult struct {
	SessionID string
	RunID     string
	FinalText string
}

func NewRunner(cfg Config) *Runner {
	return &Runner{
		bus:      cfg.Bus,
		journal:  cfg.Journal,
		sessions: cfg.Sessions,
		messages: cfg.Messages,
		tools:    cfg.Tools,
		provider: cfg.Provider,
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

	messages, err := r.initialMessages(ctx, req)
	if err != nil {
		return RunResult{}, r.fail(ctx, req, err)
	}
	maxTurns := req.MaxTurns
	if maxTurns <= 0 {
		maxTurns = 8
	}

	final := ""
	for turn := 0; turn < maxTurns; turn++ {
		providerReq := provider.Request{
			SessionID:    req.SessionID,
			RunID:        req.RunID,
			Instructions: GodeInstructions,
			Messages:     messages,
			Tools:        r.providerToolSpecs(),
		}
		events, errs := r.provider.Stream(ctx, providerReq)

		turnHadToolCall := false
		turnProducedText := false
		for events != nil || errs != nil {
			select {
			case ev, ok := <-events:
				if !ok {
					events = nil
					continue
				}
				switch ev.Kind {
				case provider.EventDelta:
					turnProducedText = true
					final += ev.Text
					r.emit(ctx, eventbus.Event{
						Kind:      eventbus.KindAssistantDelta,
						Source:    eventbus.SourceProvider,
						SessionID: req.SessionID,
						RunID:     req.RunID,
						Payload:   map[string]any{"text": ev.Text},
					})
				case provider.EventReasoningSummaryDelta:
					r.emit(ctx, eventbus.Event{
						Kind:      eventbus.KindReasoningSummaryDelta,
						Source:    eventbus.SourceProvider,
						SessionID: req.SessionID,
						RunID:     req.RunID,
						Payload:   map[string]any{"text": ev.Text},
					})
				case provider.EventReasoningSummaryDone:
					r.emit(ctx, eventbus.Event{
						Kind:      eventbus.KindReasoningSummaryCompleted,
						Source:    eventbus.SourceProvider,
						SessionID: req.SessionID,
						RunID:     req.RunID,
						Payload:   map[string]any{"text": ev.Text},
					})
				case provider.EventToolCall:
					turnHadToolCall = true
					if ev.ToolRequest == nil {
						continue
					}
					r.emit(ctx, eventbus.Event{
						Kind:      eventbus.KindToolRequested,
						Source:    eventbus.SourceProvider,
						SessionID: req.SessionID,
						RunID:     req.RunID,
						Payload: map[string]any{
							"tool_call_id": ev.ToolRequest.ID,
							"tool":         ev.ToolRequest.Name,
							"input":        ev.ToolRequest.Input,
						},
					})
					if r.tools != nil {
						messages = append(messages, provider.Message{
							Role:          provider.RoleAssistant,
							ToolCallID:    ev.ToolRequest.ID,
							ToolName:      ev.ToolRequest.Name,
							ToolArguments: ev.ToolRequest.Arguments,
						})
						result, err := r.tools.Run(ctx, tools.Call{
							ID:        ev.ToolRequest.ID,
							Name:      ev.ToolRequest.Name,
							Input:     ev.ToolRequest.Input,
							SessionID: req.SessionID,
							RunID:     req.RunID,
						})
						if err != nil {
							return RunResult{}, r.fail(ctx, req, err)
						}
						messages = append(messages, provider.Message{
							Role:       provider.RoleTool,
							ToolCallID: ev.ToolRequest.ID,
							Content:    toolResponseContent(ev.ToolRequest.Name, result),
						})
					}
				case provider.EventCompleted:
					if final == "" {
						final = ev.Text
					}
					if ev.Text != "" || final != "" {
						turnProducedText = true
					}
					r.emit(ctx, eventbus.Event{
						Kind:      eventbus.KindAssistantCompleted,
						Source:    eventbus.SourceProvider,
						SessionID: req.SessionID,
						RunID:     req.RunID,
						Payload:   map[string]any{"text": final},
					})
				}
			case err, ok := <-errs:
				if !ok {
					errs = nil
					continue
				}
				if err != nil {
					return RunResult{}, r.fail(ctx, req, err)
				}
			case <-ctx.Done():
				return RunResult{}, r.fail(ctx, req, ctx.Err())
			}
		}
		if !turnHadToolCall || turnProducedText {
			break
		}
	}
	if final == "" {
		return RunResult{}, r.fail(ctx, req, fmt.Errorf("agent stopped without final text after tool loop"))
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

func (r *Runner) initialMessages(ctx context.Context, req RunRequest) ([]provider.Message, error) {
	messages := append([]provider.Message(nil), req.Messages...)
	if req.Resume && r.messages != nil {
		prior, err := r.messages.ListBySession(ctx, req.SessionID)
		if err != nil {
			return nil, err
		}
		messages = append(providerMessages(prior), messages...)
	}
	messages = append(messages, provider.Message{Role: provider.RoleUser, Content: req.Prompt})
	return messages, nil
}

func providerMessages(messages []messagestore.Message) []provider.Message {
	out := make([]provider.Message, 0, len(messages))
	for _, msg := range messages {
		switch msg.Role {
		case messagestore.RoleUser:
			out = append(out, provider.Message{Role: provider.RoleUser, Content: msg.Text})
		case messagestore.RoleAssistant:
			out = append(out, provider.Message{Role: provider.RoleAssistant, Content: msg.Text})
		case messagestore.RoleTool:
			out = append(out, provider.Message{Role: provider.RoleTool, Content: msg.Text, ToolCallID: msg.ToolCallID})
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

func (r *Runner) fail(ctx context.Context, req RunRequest, err error) error {
	r.emit(ctx, eventbus.Event{
		Kind:      eventbus.KindRunFailed,
		Source:    eventbus.SourceAgent,
		SessionID: req.SessionID,
		RunID:     req.RunID,
		Payload:   map[string]any{"error": err.Error()},
	})
	return err
}

func (r *Runner) emit(ctx context.Context, ev eventbus.Event) eventbus.Event {
	if r.bus != nil {
		ev = r.bus.Publish(ctx, ev)
	}
	return ev
}
