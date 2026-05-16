package agent

import (
	"context"
	"encoding/json"
	"errors"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	godecommands "github.com/pandelisz/gode/internal/godex/commands"
	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/journal"
	messagestore "github.com/pandelisz/gode/internal/godex/message"
	"github.com/pandelisz/gode/internal/godex/provider"
	"github.com/pandelisz/gode/internal/godex/session"
	godeskills "github.com/pandelisz/gode/internal/godex/skills"
	"github.com/pandelisz/gode/internal/godex/tools"
)

func TestRunnerPublishesAndJournalsMockTurn(t *testing.T) {
	bus := eventbus.New(eventbus.WithSubscriberBuffer(16))
	defer bus.Close()

	store, err := journal.Open(filepath.Join(t.TempDir(), "events.jsonl"))
	if err != nil {
		t.Fatalf("journal: %v", err)
	}
	defer store.Close()

	reg := tools.NewRegistry(tools.WithEventBus(bus), tools.WithAutoApprove(true))
	reg.Register(tools.Tool{
		Name:        "echo",
		Description: "echo",
		ReadOnly:    true,
		Run: func(context.Context, tools.Call) (tools.Result, error) {
			return tools.Result{Text: "echoed"}, nil
		},
	})

	runner := NewRunner(Config{
		Bus:     bus,
		Journal: store,
		Tools:   reg,
		Provider: &scriptedProvider{streams: [][]provider.Event{
			{
				{Kind: provider.EventToolCall, ToolRequest: &provider.ToolRequest{ID: "tc1", Name: "echo", Arguments: `{}`}},
				{Kind: provider.EventCompleted},
			},
			{
				{Kind: provider.EventDelta, Text: "hello from mock"},
				{Kind: provider.EventCompleted, Text: "hello from mock"},
			},
		}},
	})

	result, err := runner.Run(context.Background(), RunRequest{
		SessionID: "s1",
		Prompt:    "hello",
	})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.FinalText != "hello from mock" {
		t.Fatalf("final = %q", result.FinalText)
	}

	events, err := store.Replay(context.Background(), journal.ReplayFilter{SessionID: "s1"})
	if err != nil {
		t.Fatalf("replay: %v", err)
	}
	kinds := map[eventbus.Kind]bool{}
	for _, ev := range events {
		kinds[ev.Kind] = true
	}
	for _, want := range []eventbus.Kind{
		eventbus.KindUserPromptSubmitted,
		eventbus.KindRunStarted,
		eventbus.KindToolRequested,
		eventbus.KindToolCompleted,
		eventbus.KindAssistantCompleted,
		eventbus.KindRunCompleted,
	} {
		if !kinds[want] {
			t.Fatalf("missing event kind %q in %#v", want, kinds)
		}
	}
}

func TestRunnerSendsGodeInstructions(t *testing.T) {
	capture := &captureProvider{finalText: "done"}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Provider: capture,
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{Prompt: "hello"}); err != nil {
		t.Fatalf("run: %v", err)
	}
	if capture.request.Instructions == "" {
		t.Fatal("instructions should be sent to provider")
	}
	for _, want := range []string{"You are gode", "Go-native coding agent", "dirty git worktree"} {
		if !strings.Contains(capture.request.Instructions, want) {
			t.Fatalf("instructions missing %q:\n%s", want, capture.request.Instructions)
		}
	}
	if len(capture.request.Messages) != 1 || capture.request.Messages[0].Content != "hello" {
		t.Fatalf("messages = %#v", capture.request.Messages)
	}
}

func TestRunnerSetsStablePromptCacheKey(t *testing.T) {
	workspace := filepath.Join(t.TempDir(), "repo")
	capture := &captureProvider{name: "openai", finalText: "done"}
	runner := NewRunner(Config{
		Bus:       eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Provider:  capture,
		Model:     "gpt-5.5",
		Workspace: workspace,
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{Prompt: "hello"}); err != nil {
		t.Fatalf("run: %v", err)
	}
	first := capture.request.PromptCacheKey
	if first == "" {
		t.Fatal("prompt cache key should be set")
	}
	if !strings.HasPrefix(first, "gode:openai:gpt-5-5:") {
		t.Fatalf("prompt cache key = %q", first)
	}
	if strings.Contains(first, workspace) {
		t.Fatalf("prompt cache key should not expose workspace path: %q", first)
	}
	if _, err := runner.Run(context.Background(), RunRequest{Prompt: "different prompt"}); err != nil {
		t.Fatalf("second run: %v", err)
	}
	if capture.request.PromptCacheKey != first {
		t.Fatalf("prompt cache key changed across same workspace/model: %q != %q", capture.request.PromptCacheKey, first)
	}
}

func TestRunnerConfiguresOpenAICompactionAndTokenEvents(t *testing.T) {
	bus := eventbus.New(eventbus.WithSubscriberBuffer(16))
	defer bus.Close()
	store, err := journal.Open(filepath.Join(t.TempDir(), "events.jsonl"))
	if err != nil {
		t.Fatalf("journal: %v", err)
	}
	defer store.Close()
	capture := &captureProvider{name: "openai", finalText: "done"}
	runner := NewRunner(Config{
		Bus:      bus,
		Journal:  store,
		Provider: capture,
		Model:    "gpt-5.5",
	})

	if _, err := runner.Run(context.Background(), RunRequest{SessionID: "s-context", RunID: "r-context", Prompt: "hello"}); err != nil {
		t.Fatalf("run: %v", err)
	}
	if !capture.request.Compaction.Enabled {
		t.Fatal("compaction should be enabled for OpenAI gpt-5.5")
	}
	if capture.request.Compaction.CompactThreshold != 600000 {
		t.Fatalf("threshold = %d", capture.request.Compaction.CompactThreshold)
	}
	events, err := store.Replay(context.Background(), journal.ReplayFilter{SessionID: "s-context", RunID: "r-context"})
	if err != nil {
		t.Fatalf("replay: %v", err)
	}
	kinds := map[eventbus.Kind]eventbus.Event{}
	for _, ev := range events {
		kinds[ev.Kind] = ev
	}
	for _, want := range []eventbus.Kind{eventbus.KindContextTokensUpdated, eventbus.KindContextCompactionConfigured} {
		if _, ok := kinds[want]; !ok {
			t.Fatalf("missing event kind %q in %#v", want, kinds)
		}
	}
	var payload struct {
		Model            string `json:"model"`
		Tokens           int    `json:"tokens"`
		ContextWindow    int    `json:"context_window"`
		CompactThreshold int    `json:"compact_threshold"`
	}
	if err := kinds[eventbus.KindContextCompactionConfigured].DecodePayload(&payload); err != nil {
		t.Fatalf("decode payload: %v", err)
	}
	if payload.Model != "gpt-5.5" || payload.ContextWindow != 1050000 || payload.CompactThreshold != 600000 || payload.Tokens <= 0 {
		t.Fatalf("payload = %#v", payload)
	}
}

func TestRunnerForceCompactsAndRetriesContextLengthExceeded(t *testing.T) {
	capture := &contextLengthRetryProvider{
		compactingCaptureProvider: compactingCaptureProvider{
			captureProvider: captureProvider{name: "openai", finalText: "done"},
			output:          []json.RawMessage{json.RawMessage(`{"id":"msg_compacted","type":"message","role":"user","content":[{"type":"input_text","text":"compacted context"}]}`)},
		},
	}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Provider: capture,
		Model:    "gpt-5.5",
	})
	defer runner.bus.Close()

	result, err := runner.Run(context.Background(), RunRequest{
		SessionID: "s-context-length",
		RunID:     "r-context-length",
		Messages:  []provider.Message{{Role: provider.RoleUser, Content: "existing context"}},
		Prompt:    "continue",
	})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.FinalText != "done" {
		t.Fatalf("final = %q", result.FinalText)
	}
	if capture.streamCalls != 2 {
		t.Fatalf("stream calls = %d", capture.streamCalls)
	}
	if len(capture.compactRequests) != 1 {
		t.Fatalf("compact requests = %d", len(capture.compactRequests))
	}
	if len(capture.request.InputItems) == 0 || !strings.Contains(string(capture.request.InputItems[0].RawJSON), "compacted context") {
		t.Fatalf("retry did not use compacted input items: %#v", capture.request.InputItems)
	}
}

func TestRunnerRepairsOrphanToolOutputCompactionAndRetries(t *testing.T) {
	bus := eventbus.New(eventbus.WithSubscriberBuffer(32))
	defer bus.Close()
	store, err := journal.Open(filepath.Join(t.TempDir(), "events.jsonl"))
	if err != nil {
		t.Fatalf("journal: %v", err)
	}
	defer store.Close()
	capture := &compactingCaptureProvider{
		captureProvider: captureProvider{name: "openai", finalText: "done"},
		output:          []json.RawMessage{json.RawMessage(`{"id":"msg_compacted","type":"message","role":"user","content":[{"type":"input_text","text":"compacted context"}]}`)},
	}
	runner := NewRunner(Config{
		Bus:                   bus,
		Journal:               store,
		Provider:              capture,
		Model:                 "gpt-5.5",
		AutoCompactTokenLimit: 1,
	})

	_, err = runner.Run(context.Background(), RunRequest{
		SessionID: "s-repair-compact",
		RunID:     "r-repair-compact",
		Messages: []provider.Message{
			{RawJSON: json.RawMessage(`{"type":"function_call_output","call_id":"call_missing","output":"orphan output"}`)},
			{Role: provider.RoleUser, Content: "safe context"},
		},
		Prompt: "continue",
	})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if len(capture.compactRequests) != 1 {
		t.Fatalf("compact requests = %d", len(capture.compactRequests))
	}
	if hasFunctionCallOutput(capture.compactRequests[0].Messages, "call_missing") {
		t.Fatalf("compact request still contains orphan output: %#v", capture.compactRequests[0].Messages)
	}
	events, err := store.Replay(context.Background(), journal.ReplayFilter{SessionID: "s-repair-compact", RunID: "r-repair-compact"})
	if err != nil {
		t.Fatalf("replay: %v", err)
	}
	foundRepair := false
	for _, ev := range events {
		if ev.Kind == eventbus.KindContextCompactionRepaired {
			foundRepair = true
		}
		if ev.Kind == eventbus.KindContextCompactionFailed {
			t.Fatalf("compaction should not publish failure after repair: %#v", ev)
		}
	}
	if !foundRepair {
		t.Fatalf("missing repair event in %#v", events)
	}
}

func TestRunnerLocallyPrunesWhenRemoteCompactionDecodeFails(t *testing.T) {
	capture := &compactingCaptureProvider{
		captureProvider: captureProvider{name: "openai", finalText: "done"},
		compactErrors: []error{
			errors.New("expected destination type of 'string' or '[]byte' for responses with content-type '' that is not 'application/json'"),
		},
	}
	runner := NewRunner(Config{
		Bus:                   eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Provider:              capture,
		Model:                 "gpt-5.5",
		AutoCompactTokenLimit: 50,
	})
	defer runner.bus.Close()

	result, err := runner.Run(context.Background(), RunRequest{
		SessionID: "s-local-prune",
		RunID:     "r-local-prune",
		Messages: []provider.Message{
			{Role: provider.RoleUser, Content: strings.Repeat("old context ", 2000)},
			{Role: provider.RoleAssistant, Content: strings.Repeat("old answer ", 2000)},
		},
		Prompt: "continue",
	})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.FinalText != "done" {
		t.Fatalf("final = %q", result.FinalText)
	}
	if len(capture.compactRequests) != 1 {
		t.Fatalf("compact requests = %d", len(capture.compactRequests))
	}
	if len(capture.request.Messages) != 2 {
		t.Fatalf("provider messages after local prune = %#v", capture.request.Messages)
	}
	if !strings.Contains(string(capture.request.Messages[0].RawJSON), provider.LocalPruneMarkerText) {
		t.Fatalf("missing local prune marker: %#v", capture.request.Messages[0])
	}
	if capture.request.Messages[1].Content != "continue" {
		t.Fatalf("latest prompt should be retained: %#v", capture.request.Messages[1])
	}
}

func TestRunnerRepairsOrphanInputItemsBeforeStreaming(t *testing.T) {
	capture := &captureProvider{name: "openai", finalText: "done"}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Provider: capture,
		Model:    "gpt-5.5",
	})
	defer runner.bus.Close()

	_, err := runner.Run(context.Background(), RunRequest{
		SessionID:     "s-input-repair",
		RunID:         "r-input-repair",
		ReplacePrompt: true,
		InputItems: []provider.Item{
			{Kind: provider.ItemFunctionOut, ToolCallID: "call_missing", Text: "orphan"},
			{Kind: provider.ItemMessage, Role: "user", Text: "continue"},
		},
	})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if len(capture.request.InputItems) != 1 {
		t.Fatalf("input items = %#v", capture.request.InputItems)
	}
	if capture.request.InputItems[0].Kind != provider.ItemMessage || capture.request.InputItems[0].Text != "continue" {
		t.Fatalf("orphan function output should be removed: %#v", capture.request.InputItems)
	}
	if hasFunctionCallOutput(capture.request.Messages, "call_missing") {
		t.Fatalf("messages still contain orphan output: %#v", capture.request.Messages)
	}
}

func TestRunnerEmitsActualTokenUsageAfterCompletion(t *testing.T) {
	bus := eventbus.New(eventbus.WithSubscriberBuffer(16))
	defer bus.Close()
	store, err := journal.Open(filepath.Join(t.TempDir(), "events.jsonl"))
	if err != nil {
		t.Fatalf("journal: %v", err)
	}
	defer store.Close()
	runner := NewRunner(Config{
		Bus:      bus,
		Journal:  store,
		Provider: &captureProvider{name: "openai", finalText: "done", usage: provider.TokenUsage{InputTokens: 25, OutputTokens: 17, TotalTokens: 42}},
		Model:    "gpt-5.5",
	})

	if _, err := runner.Run(context.Background(), RunRequest{SessionID: "s-usage", RunID: "r-usage", Prompt: strings.Repeat("long prompt ", 100)}); err != nil {
		t.Fatalf("run: %v", err)
	}
	events, err := store.Replay(context.Background(), journal.ReplayFilter{SessionID: "s-usage", RunID: "r-usage", Kinds: []eventbus.Kind{eventbus.KindContextTokensUpdated}})
	if err != nil {
		t.Fatalf("replay: %v", err)
	}
	var actual struct {
		Tokens         int    `json:"tokens"`
		InputTokens    int64  `json:"input_tokens"`
		OutputTokens   int64  `json:"output_tokens"`
		TotalTokens    int64  `json:"total_tokens"`
		CountSource    string `json:"count_source"`
		UsageIncrement bool   `json:"usage_increment"`
	}
	for _, ev := range events {
		if ev.Source != eventbus.SourceProvider {
			continue
		}
		if err := ev.DecodePayload(&actual); err != nil {
			t.Fatalf("decode payload: %v", err)
		}
	}
	if actual.CountSource != "response" || !actual.UsageIncrement || actual.Tokens != 42 || actual.InputTokens != 25 || actual.OutputTokens != 17 || actual.TotalTokens != 42 {
		t.Fatalf("actual usage payload = %#v", actual)
	}
}

func TestRunnerOmitsCompactionWhenDisabled(t *testing.T) {
	capture := &captureProvider{name: "openai", finalText: "done"}
	runner := NewRunner(Config{
		Bus:                   eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Provider:              capture,
		Model:                 "gpt-5.5",
		DisableAutoCompaction: true,
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{Prompt: "hello"}); err != nil {
		t.Fatalf("run: %v", err)
	}
	if capture.request.Compaction.Enabled {
		t.Fatalf("compaction should be disabled: %#v", capture.request.Compaction)
	}
	if capture.request.Compaction.CompactThreshold != 600000 {
		t.Fatalf("threshold should still be discoverable, got %d", capture.request.Compaction.CompactThreshold)
	}
}

func TestRunnerPrependsContextMessages(t *testing.T) {
	capture := &captureProvider{finalText: "done"}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Provider: capture,
		ContextMessages: []provider.Message{{
			Role:    provider.RoleSystem,
			Content: "<repo-context-file path=\"AGENTS.md\">rules</repo-context-file>",
		}},
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{Prompt: "hello"}); err != nil {
		t.Fatalf("run: %v", err)
	}
	if len(capture.request.Messages) != 2 {
		t.Fatalf("messages = %#v", capture.request.Messages)
	}
	if capture.request.Messages[0].Role != provider.RoleSystem || !strings.Contains(capture.request.Messages[0].Content, "AGENTS.md") {
		t.Fatalf("context message = %#v", capture.request.Messages[0])
	}
	if capture.request.Messages[1].Role != provider.RoleUser || capture.request.Messages[1].Content != "hello" {
		t.Fatalf("user message = %#v", capture.request.Messages[1])
	}
}

func TestRunnerInjectsRoadmapContextOnlyWhenProvided(t *testing.T) {
	capture := &captureProvider{finalText: "done"}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Provider: capture,
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{Prompt: "hello"}); err != nil {
		t.Fatalf("run without roadmap context: %v", err)
	}
	if len(capture.request.Messages) != 1 || capture.request.Messages[0].Content != "hello" {
		t.Fatalf("normal messages = %#v", capture.request.Messages)
	}

	if _, err := runner.Run(context.Background(), RunRequest{
		Prompt:         "continue",
		RoadmapContext: "roadmap context",
	}); err != nil {
		t.Fatalf("run with roadmap context: %v", err)
	}
	if len(capture.request.Messages) != 2 {
		t.Fatalf("roadmap messages = %#v", capture.request.Messages)
	}
	if capture.request.Messages[0].Role != provider.RoleSystem || capture.request.Messages[0].Content != "roadmap context" {
		t.Fatalf("roadmap context message = %#v", capture.request.Messages[0])
	}
	if capture.request.Messages[1].Role != provider.RoleUser || capture.request.Messages[1].Content != "continue" {
		t.Fatalf("user message = %#v", capture.request.Messages[1])
	}
}

func TestRunnerInjectsInvokedSkillsAndCleansPrompt(t *testing.T) {
	capture := &captureProvider{finalText: "done"}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Provider: capture,
		Skills: []godeskills.Skill{{
			Name: "go-tests",
			Body: "Run Go tests before reporting completion.",
		}},
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{Prompt: "$go-tests please check this"}); err != nil {
		t.Fatalf("run: %v", err)
	}
	if len(capture.request.Messages) != 3 {
		t.Fatalf("messages = %#v", capture.request.Messages)
	}
	if capture.request.Messages[0].Role != provider.RoleSystem || !strings.Contains(capture.request.Messages[0].Content, godeskills.SkillsInstructionsOpenTag) {
		t.Fatalf("available skills message = %#v", capture.request.Messages[0])
	}
	if capture.request.Messages[1].Role != provider.RoleUser || !strings.Contains(capture.request.Messages[1].Content, `<name>go-tests</name>`) {
		t.Fatalf("skill message = %#v", capture.request.Messages[1])
	}
	if capture.request.Messages[2].Role != provider.RoleUser || capture.request.Messages[2].Content != "please check this" {
		t.Fatalf("user message = %#v", capture.request.Messages[2])
	}
}

func TestRunnerInjectsStructuredSkillSelection(t *testing.T) {
	capture := &captureProvider{finalText: "done"}
	skillPath := "/skills/go-tests/SKILL.md"
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Provider: capture,
		Skills: []godeskills.Skill{{
			Name: "go-tests",
			Path: skillPath,
			Body: "Run Go tests before reporting completion.",
		}},
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{
		Prompt:          "please check this",
		SkillSelections: []godeskills.InvocationSelection{{Path: skillPath}},
	}); err != nil {
		t.Fatalf("run: %v", err)
	}
	if len(capture.request.Messages) != 3 {
		t.Fatalf("messages = %#v", capture.request.Messages)
	}
	if capture.request.Messages[1].Role != provider.RoleUser || !strings.Contains(capture.request.Messages[1].Content, `<path>`+skillPath+`</path>`) {
		t.Fatalf("skill message = %#v", capture.request.Messages[1])
	}
	if capture.request.Messages[2].Role != provider.RoleUser || capture.request.Messages[2].Content != "please check this" {
		t.Fatalf("user message = %#v", capture.request.Messages[2])
	}
}

func TestRunnerSkipsDisabledInvokedSkillAndKeepsPromptDiagnostic(t *testing.T) {
	capture := &captureProvider{finalText: "done"}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Provider: capture,
		Skills: []godeskills.Skill{{
			Name: "go-tests",
			Path: "/skills/go-tests/SKILL.md",
			Body: "Run Go tests before reporting completion.",
		}},
		SkillsConfig: godeskills.Config{Rules: []godeskills.ConfigRule{{Name: "go-tests", Enabled: false}}},
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{Prompt: "$go-tests please check this"}); err != nil {
		t.Fatalf("run: %v", err)
	}
	if len(capture.request.Messages) != 2 {
		t.Fatalf("messages = %#v", capture.request.Messages)
	}
	if capture.request.Messages[0].Role != provider.RoleSystem || !strings.Contains(capture.request.Messages[0].Content, "Skill diagnostic") || !strings.Contains(capture.request.Messages[0].Content, "disabled") {
		t.Fatalf("diagnostic message = %#v", capture.request.Messages[0])
	}
	if strings.Contains(capture.request.Messages[0].Content, `<skill name="go-tests">`) {
		t.Fatalf("disabled skill body leaked into diagnostic: %#v", capture.request.Messages[0])
	}
	if capture.request.Messages[1].Role != provider.RoleUser || capture.request.Messages[1].Content != "$go-tests please check this" {
		t.Fatalf("user prompt = %#v", capture.request.Messages[1])
	}
}

func TestRunnerLoadsActiveSkillSettingsPerRun(t *testing.T) {
	active := godeskills.Config{Rules: []godeskills.ConfigRule{{Name: "go-tests", Enabled: false}}}
	capture := &captureProvider{finalText: "done"}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Provider: capture,
		Skills: []godeskills.Skill{{
			Name: "go-tests",
			Body: "Run Go tests before reporting completion.",
		}},
		LoadSkillsConfig: func(context.Context) (godeskills.Config, error) {
			return active, nil
		},
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{Prompt: "$go-tests one"}); err != nil {
		t.Fatalf("first run: %v", err)
	}
	if !strings.Contains(capture.request.Messages[0].Content, "disabled") {
		t.Fatalf("first run should see disabled skill: %#v", capture.request.Messages)
	}
	active = godeskills.Config{Rules: []godeskills.ConfigRule{{Name: "go-tests", Enabled: true}}}
	if _, err := runner.Run(context.Background(), RunRequest{Prompt: "$go-tests two"}); err != nil {
		t.Fatalf("second run: %v", err)
	}
	if len(capture.request.Messages) != 3 || !strings.Contains(capture.request.Messages[1].Content, `<name>go-tests</name>`) || capture.request.Messages[2].Content != "two" {
		t.Fatalf("second run messages = %#v", capture.request.Messages)
	}
}

func TestRunnerExpandsSlashCommandsBeforeProviderRequest(t *testing.T) {
	capture := &captureProvider{finalText: "done"}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Provider: capture,
		Commands: []godecommands.Command{{
			ID:           "project:test",
			Scope:        "project",
			Prompt:       "Run $TARGET tests",
			Placeholders: []string{"TARGET"},
		}},
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{Prompt: "/test TARGET=api with coverage"}); err != nil {
		t.Fatalf("run: %v", err)
	}
	if len(capture.request.Messages) != 1 {
		t.Fatalf("messages = %#v", capture.request.Messages)
	}
	if capture.request.Messages[0].Content != "Run api tests\n\nwith coverage" {
		t.Fatalf("prompt = %q", capture.request.Messages[0].Content)
	}
}

func TestRunnerDoesNotTreatAbsolutePathAsSlashCommand(t *testing.T) {
	capture := &captureProvider{finalText: "done"}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Provider: capture,
		Commands: []godecommands.Command{{
			ID:     "project:Users",
			Scope:  "project",
			Prompt: "wrong",
		}},
	})
	defer runner.bus.Close()

	prompt := "/Users/pz/file.go is a path"
	if _, err := runner.Run(context.Background(), RunRequest{Prompt: prompt}); err != nil {
		t.Fatalf("run: %v", err)
	}
	if len(capture.request.Messages) != 1 || capture.request.Messages[0].Content != prompt {
		t.Fatalf("messages = %#v", capture.request.Messages)
	}
}

func TestRunnerCarriesFunctionCallBeforeToolOutput(t *testing.T) {
	reg := tools.NewRegistry(tools.WithAutoApprove(true))
	reg.Register(tools.Tool{
		Name:        "echo",
		Description: "echo",
		ReadOnly:    true,
		Run: func(context.Context, tools.Call) (tools.Result, error) {
			return tools.Result{Text: "echoed"}, nil
		},
	})
	script := &scriptedProvider{
		streams: [][]provider.Event{
			{
				{
					Kind: provider.EventToolCall,
					ToolRequest: &provider.ToolRequest{
						ID:        "call_abc",
						Name:      "echo",
						Input:     map[string]any{"text": "hello"},
						Arguments: `{"text":"hello"}`,
					},
				},
				{Kind: provider.EventCompleted},
			},
			{
				{Kind: provider.EventDelta, Text: "done"},
				{Kind: provider.EventCompleted, Text: "done"},
			},
		},
	}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Tools:    reg,
		Provider: script,
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{Prompt: "hello"}); err != nil {
		t.Fatalf("run: %v", err)
	}
	if len(script.requests) != 2 {
		t.Fatalf("requests = %d, want 2", len(script.requests))
	}
	messages := script.requests[1].Messages
	if len(messages) != 3 {
		t.Fatalf("second request messages = %#v", messages)
	}
	if messages[1].Role != provider.RoleAssistant || messages[1].ToolCallID != "call_abc" || messages[1].ToolName != "echo" || messages[1].ToolArguments != `{"text":"hello"}` {
		t.Fatalf("assistant function call message = %#v", messages[1])
	}
	if messages[2].Role != provider.RoleTool || messages[2].ToolCallID != "call_abc" || !strings.Contains(messages[2].Content, "echoed") {
		t.Fatalf("tool output message = %#v", messages[2])
	}
}

func TestRunnerRunsParallelToolCallsAndPreservesResponseOrder(t *testing.T) {
	var running int32
	var maxRunning int32
	reg := tools.NewRegistry(tools.WithAutoApprove(true))
	for _, name := range []string{"first", "second"} {
		name := name
		reg.Register(tools.Tool{
			Name:        name,
			Description: name,
			ReadOnly:    true,
			Run: func(context.Context, tools.Call) (tools.Result, error) {
				now := atomic.AddInt32(&running, 1)
				for {
					seen := atomic.LoadInt32(&maxRunning)
					if now <= seen || atomic.CompareAndSwapInt32(&maxRunning, seen, now) {
						break
					}
				}
				time.Sleep(50 * time.Millisecond)
				atomic.AddInt32(&running, -1)
				return tools.Result{Text: name + " output"}, nil
			},
		})
	}
	script := &scriptedProvider{
		streams: [][]provider.Event{
			{
				{Kind: provider.EventToolCall, ToolRequest: &provider.ToolRequest{ID: "call_1", Name: "first", Arguments: `{}`}},
				{Kind: provider.EventToolCall, ToolRequest: &provider.ToolRequest{ID: "call_2", Name: "second", Arguments: `{}`}},
				{Kind: provider.EventCompleted},
			},
			{
				{Kind: provider.EventDelta, Text: "done"},
				{Kind: provider.EventCompleted, Text: "done"},
			},
		},
	}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Tools:    reg,
		Provider: script,
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{Prompt: "hello"}); err != nil {
		t.Fatalf("run: %v", err)
	}
	if maxRunning < 2 {
		t.Fatalf("tool calls did not overlap, max running = %d", maxRunning)
	}
	messages := script.requests[1].Messages
	if len(messages) != 5 {
		t.Fatalf("second request messages = %#v", messages)
	}
	want := []struct {
		role provider.Role
		id   string
		name string
	}{
		{provider.RoleUser, "", ""},
		{provider.RoleAssistant, "call_1", "first"},
		{provider.RoleAssistant, "call_2", "second"},
		{provider.RoleTool, "call_1", ""},
		{provider.RoleTool, "call_2", ""},
	}
	for i, want := range want {
		if messages[i].Role != want.role || messages[i].ToolCallID != want.id || messages[i].ToolName != want.name {
			t.Fatalf("message %d = %#v, want role=%s id=%s name=%s", i, messages[i], want.role, want.id, want.name)
		}
	}
}

func TestRunnerFeedsToolFailureBackToModel(t *testing.T) {
	reg := tools.NewRegistry(tools.WithAutoApprove(true))
	reg.Register(tools.Tool{
		Name:        "apply_patch",
		Description: "patch",
		ReadOnly:    false,
		Run: func(context.Context, tools.Call) (tools.Result, error) {
			return tools.Result{Text: "error: corrupt patch at line 4"}, errors.New("failed to apply patch: exit status 128")
		},
	})
	script := &scriptedProvider{
		streams: [][]provider.Event{
			{
				{
					Kind: provider.EventToolCall,
					ToolRequest: &provider.ToolRequest{
						ID:        "call_patch",
						Name:      "apply_patch",
						Input:     map[string]any{"patch": "bad"},
						Arguments: `{"patch":"bad"}`,
					},
				},
				{Kind: provider.EventCompleted},
			},
			{
				{Kind: provider.EventDelta, Text: "recovered"},
				{Kind: provider.EventCompleted, Text: "recovered"},
			},
		},
	}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Tools:    reg,
		Provider: script,
	})
	defer runner.bus.Close()

	result, err := runner.Run(context.Background(), RunRequest{Prompt: "patch this"})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.FinalText != "recovered" {
		t.Fatalf("final = %q", result.FinalText)
	}
	if len(script.requests) != 2 {
		t.Fatalf("requests = %d, want 2", len(script.requests))
	}
	messages := script.requests[1].Messages
	if len(messages) != 3 {
		t.Fatalf("second request messages = %#v", messages)
	}
	if messages[2].Role != provider.RoleTool || messages[2].ToolCallID != "call_patch" {
		t.Fatalf("tool output message = %#v", messages[2])
	}
	for _, want := range []string{"Tool apply_patch failed", "failed to apply patch: exit status 128", "error: corrupt patch at line 4"} {
		if !strings.Contains(messages[2].Content, want) {
			t.Fatalf("tool output missing %q:\n%s", want, messages[2].Content)
		}
	}
}

func TestRunnerPersistsSessionAndMessages(t *testing.T) {
	dataDir := t.TempDir()
	sessionStore := openSessionStore(t, dataDir)
	messageStore := messagestore.Open(dataDir)
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Sessions: sessionStore,
		Messages: messageStore,
		Provider: &scriptedProvider{streams: [][]provider.Event{{
			{Kind: provider.EventDelta, Text: "done"},
			{
				Kind:  provider.EventCompleted,
				Text:  "done",
				Usage: provider.TokenUsage{InputTokens: 12, OutputTokens: 8, TotalTokens: 20},
			},
		}}},
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{SessionID: "s-persist", Prompt: "hello"}); err != nil {
		t.Fatalf("run: %v", err)
	}
	messages, err := messageStore.ListBySession(context.Background(), "s-persist")
	if err != nil {
		t.Fatalf("list messages: %v", err)
	}
	if len(messages) != 2 || messages[0].Text != "hello" || messages[1].Text != "done" {
		t.Fatalf("messages = %#v", messages)
	}
	stored, ok, err := sessionStore.Get(context.Background(), "s-persist")
	if err != nil {
		t.Fatalf("get session: %v", err)
	}
	if !ok || stored.Title != "hello" || stored.MessageCount != 2 {
		t.Fatalf("session = %#v ok=%v", stored, ok)
	}
	if stored.PromptTokens != 12 || stored.CompletionTokens != 8 {
		t.Fatalf("session token usage = %#v", stored)
	}
}

func TestRunnerPublishesReasoningSummaryEvents(t *testing.T) {
	bus := eventbus.New(eventbus.WithSubscriberBuffer(16))
	defer bus.Close()

	store, err := journal.Open(filepath.Join(t.TempDir(), "events.jsonl"))
	if err != nil {
		t.Fatalf("journal: %v", err)
	}
	defer store.Close()

	runner := NewRunner(Config{
		Bus:     bus,
		Journal: store,
		Provider: &scriptedProvider{streams: [][]provider.Event{{
			{Kind: provider.EventReasoningSummaryDelta, Text: "Checking files"},
			{Kind: provider.EventReasoningSummaryDone, Text: "Checking files before editing."},
			{Kind: provider.EventDelta, Text: "done"},
			{Kind: provider.EventCompleted, Text: "done"},
		}}},
	})

	if _, err := runner.Run(context.Background(), RunRequest{SessionID: "s-reasoning", Prompt: "hello"}); err != nil {
		t.Fatalf("run: %v", err)
	}

	events, err := store.Replay(context.Background(), journal.ReplayFilter{SessionID: "s-reasoning"})
	if err != nil {
		t.Fatalf("replay: %v", err)
	}
	kinds := map[eventbus.Kind]string{}
	for _, ev := range events {
		var payload struct {
			Text string `json:"text"`
		}
		_ = ev.DecodePayload(&payload)
		kinds[ev.Kind] = payload.Text
	}
	if kinds[eventbus.KindReasoningSummaryDelta] != "Checking files" {
		t.Fatalf("reasoning delta = %q", kinds[eventbus.KindReasoningSummaryDelta])
	}
	if kinds[eventbus.KindReasoningSummaryCompleted] != "Checking files before editing." {
		t.Fatalf("reasoning completed = %q", kinds[eventbus.KindReasoningSummaryCompleted])
	}
}

func openSessionStore(t *testing.T, dataDir string) *session.Store {
	t.Helper()
	store, err := session.Open(dataDir)
	if err != nil {
		t.Fatalf("session store: %v", err)
	}
	return store
}

type captureProvider struct {
	name      string
	request   provider.Request
	finalText string
	usage     provider.TokenUsage
}

func (p *captureProvider) Name() string {
	if p.name != "" {
		return p.name
	}
	return "capture"
}

func (p *captureProvider) Stream(_ context.Context, req provider.Request) (<-chan provider.Event, <-chan error) {
	p.request = req
	events := make(chan provider.Event, 2)
	errs := make(chan error)
	events <- provider.Event{Kind: provider.EventDelta, Text: p.finalText}
	events <- provider.Event{Kind: provider.EventCompleted, Text: p.finalText, Usage: p.usage}
	close(events)
	close(errs)
	return events, errs
}

type compactingCaptureProvider struct {
	captureProvider
	compactRequest  provider.CompactRequest
	compactRequests []provider.CompactRequest
	compactErrors   []error
	output          []json.RawMessage
}

func (p *compactingCaptureProvider) Compact(_ context.Context, req provider.CompactRequest) (provider.CompactResult, error) {
	p.compactRequest = req
	p.compactRequests = append(p.compactRequests, req)
	if index := len(p.compactRequests) - 1; index < len(p.compactErrors) && p.compactErrors[index] != nil {
		return provider.CompactResult{}, p.compactErrors[index]
	}
	return provider.CompactResult{ID: "resp_compact", Output: p.output}, nil
}

type contextLengthRetryProvider struct {
	compactingCaptureProvider
	streamCalls int
}

func (p *contextLengthRetryProvider) Stream(ctx context.Context, req provider.Request) (<-chan provider.Event, <-chan error) {
	p.streamCalls++
	if p.streamCalls == 1 {
		p.request = req
		events := make(chan provider.Event)
		errs := make(chan error, 1)
		close(events)
		errs <- errors.New(`OpenAI stream request failed
error: received error while streaming: {"type":"invalid_request_error","code":"context_length_exceeded","message":"Your input exceeds the context window of this model. Please adjust your input and try again.","param":"input"}`)
		close(errs)
		return events, errs
	}
	return p.compactingCaptureProvider.Stream(ctx, req)
}

func hasFunctionCallOutput(messages []provider.Message, callID string) bool {
	for _, msg := range messages {
		if msg.Role == provider.RoleTool && msg.ToolCallID == callID {
			return true
		}
		if len(msg.RawJSON) == 0 {
			continue
		}
		var object map[string]json.RawMessage
		if err := json.Unmarshal(msg.RawJSON, &object); err != nil {
			continue
		}
		var typ string
		_ = json.Unmarshal(object["type"], &typ)
		var gotCallID string
		_ = json.Unmarshal(object["call_id"], &gotCallID)
		if typ == "function_call_output" && gotCallID == callID {
			return true
		}
	}
	return false
}

type scriptedProvider struct {
	requests []provider.Request
	streams  [][]provider.Event
}

func (p *scriptedProvider) Name() string {
	return "scripted"
}

func (p *scriptedProvider) Stream(_ context.Context, req provider.Request) (<-chan provider.Event, <-chan error) {
	p.requests = append(p.requests, req)
	index := len(p.requests) - 1
	buffer := 8
	if index < len(p.streams) && len(p.streams[index]) > buffer {
		buffer = len(p.streams[index])
	}
	events := make(chan provider.Event, buffer)
	errs := make(chan error)
	if index < len(p.streams) {
		for _, ev := range p.streams[index] {
			events <- ev
		}
	}
	close(events)
	close(errs)
	return events, errs
}
