package godex

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/goals"
	"github.com/pandelisz/gode/internal/godex/journal"
	"github.com/pandelisz/gode/internal/godex/memory"
	messagestore "github.com/pandelisz/gode/internal/godex/message"
	"github.com/pandelisz/gode/internal/godex/provider"
	"github.com/pandelisz/gode/internal/godex/session"
	godeskills "github.com/pandelisz/gode/internal/godex/skills"
	"github.com/pandelisz/gode/internal/godex/tools"
)

func TestNewAppWiresBroadCoreWithMockProvider(t *testing.T) {
	app, err := New(context.Background(), Config{
		Workspace:   filepath.Join(t.TempDir(), "workspace"),
		DataDir:     t.TempDir(),
		Provider:    "mock",
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("new app: %v", err)
	}
	defer app.Close(context.Background())

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	events := app.Bus.Subscribe(ctx, eventbus.Filter{Kinds: []eventbus.Kind{eventbus.KindRunCompleted}})

	result, err := app.RunPrompt(context.Background(), "hello")
	if err != nil {
		t.Fatalf("run: %v", err)
	}

	select {
	case ev := <-events:
		if ev.Kind != eventbus.KindRunCompleted {
			t.Fatalf("kind = %q", ev.Kind)
		}
	default:
		t.Fatal("expected run completed event")
	}

	messages, err := app.Messages.ListBySession(context.Background(), result.SessionID)
	if err != nil {
		t.Fatalf("messages: %v", err)
	}
	if len(messages) != 2 || messages[0].Text != "hello" || messages[1].Text != "mock response" {
		t.Fatalf("messages = %#v", messages)
	}
	session, ok, err := app.Sessions.Get(context.Background(), result.SessionID)
	if err != nil {
		t.Fatalf("session: %v", err)
	}
	if !ok || session.MessageCount != 2 {
		t.Fatalf("session = %#v ok=%v", session, ok)
	}
	if app.LSP == nil {
		t.Fatal("lsp manager should be wired")
	}
	if app.Goals == nil {
		t.Fatal("goal runtime should be wired")
	}
}

func TestNewAppShellToolHasDefaultBuiltins(t *testing.T) {
	workspace := filepath.Join(t.TempDir(), "workspace")
	if err := os.MkdirAll(workspace, 0o700); err != nil {
		t.Fatalf("mkdir workspace: %v", err)
	}
	if err := os.WriteFile(filepath.Join(workspace, "file.json"), []byte(`{"name":"gode"}`+"\n"), 0o600); err != nil {
		t.Fatalf("write json: %v", err)
	}
	app, err := New(context.Background(), Config{
		Workspace:   workspace,
		DataDir:     t.TempDir(),
		HomeDir:     t.TempDir(),
		Provider:    "mock",
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("new app: %v", err)
	}
	defer app.Close(context.Background())

	result, err := app.Tools.Run(context.Background(), tools.Call{Name: "shell", Input: map[string]any{
		"command": "jq -r .name file.json; gode_list_files .",
	}})
	if err != nil {
		t.Fatalf("run shell: %v", err)
	}
	if result.Error != "" || !strings.Contains(result.Text, "gode") || !strings.Contains(result.Text, "file.json") {
		t.Fatalf("shell result = %#v", result)
	}
}

func TestAppLoadsPatchToolOnlyForGPTModels(t *testing.T) {
	app, err := New(context.Background(), Config{
		Workspace:   filepath.Join(t.TempDir(), "workspace"),
		DataDir:     t.TempDir(),
		HomeDir:     t.TempDir(),
		Model:       "gpt-5.5",
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("new gpt app: %v", err)
	}
	defer app.Close(context.Background())
	if !appHasTool(app, "apply_patch") {
		t.Fatalf("gpt tools should include apply_patch: %#v", app.Tools.Specs())
	}
	for _, name := range []string{"edit", "multi_edit", "write_file"} {
		if appHasTool(app, name) {
			t.Fatalf("gpt tools should not include %s: %#v", name, app.Tools.Specs())
		}
	}
}

func TestAppLoadsEditToolsOnlyForNonGPTModels(t *testing.T) {
	app, err := New(context.Background(), Config{
		Workspace:   filepath.Join(t.TempDir(), "workspace"),
		DataDir:     t.TempDir(),
		HomeDir:     t.TempDir(),
		Provider:    ProviderAnthropic,
		Model:       "claude-sonnet-4-6",
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("new claude app: %v", err)
	}
	defer app.Close(context.Background())
	for _, name := range []string{"edit", "multi_edit", "write_file"} {
		if !appHasTool(app, name) {
			t.Fatalf("non-gpt tools should include %s: %#v", name, app.Tools.Specs())
		}
	}
	if appHasTool(app, "apply_patch") {
		t.Fatalf("non-gpt tools should not include apply_patch: %#v", app.Tools.Specs())
	}
}

func TestSetModelReasoningRebuildsEditToolSurface(t *testing.T) {
	app, err := New(context.Background(), Config{
		Workspace:   filepath.Join(t.TempDir(), "workspace"),
		DataDir:     t.TempDir(),
		HomeDir:     t.TempDir(),
		Provider:    ProviderAnthropic,
		Model:       "claude-sonnet-4-6",
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("new app: %v", err)
	}
	defer app.Close(context.Background())
	if !appHasTool(app, "edit") || appHasTool(app, "apply_patch") {
		t.Fatalf("initial edit surface = %#v", app.Tools.Specs())
	}
	if err := app.SetModelReasoning("gpt-5.5", ReasoningMedium); err != nil {
		t.Fatalf("set model: %v", err)
	}
	if !appHasTool(app, "apply_patch") || appHasTool(app, "edit") || appHasTool(app, "multi_edit") || appHasTool(app, "write_file") {
		t.Fatalf("updated edit surface = %#v", app.Tools.Specs())
	}
}

func TestAppGoalMethodsPublishEvents(t *testing.T) {
	app, err := New(context.Background(), Config{
		Workspace:   filepath.Join(t.TempDir(), "workspace"),
		DataDir:     t.TempDir(),
		Provider:    "mock",
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("new app: %v", err)
	}
	defer app.Close(context.Background())

	events := app.Bus.Subscribe(context.Background(), eventbus.Filter{SessionID: "s1"})
	goal, err := app.SetGoal(context.Background(), goals.SetRequest{SessionID: "s1", Objective: "ship it"})
	if err != nil {
		t.Fatalf("set goal: %v", err)
	}
	if goal.Status != goals.StatusActive {
		t.Fatalf("goal = %#v", goal)
	}
	ev := <-events
	if ev.Kind != eventbus.KindGoalUpdated {
		t.Fatalf("event = %#v", ev)
	}
	var payload struct {
		SessionID       string `json:"session_id"`
		GoalID          string `json:"goal_id"`
		Status          string `json:"status"`
		TokensUsed      int64  `json:"tokens_used"`
		TimeUsedSeconds int64  `json:"time_used_seconds"`
	}
	if err := ev.DecodePayload(&payload); err != nil {
		t.Fatalf("decode payload: %v", err)
	}
	if payload.SessionID != "s1" || payload.GoalID == "" || payload.Status != "active" {
		t.Fatalf("payload = %#v", payload)
	}
	if _, err := app.SetGoal(context.Background(), goals.SetRequest{SessionID: "s1", Objective: "replace"}); !errors.Is(err, goals.ErrActiveGoalExists) {
		t.Fatalf("replace err = %v", err)
	}
	if err := app.ClearGoal(context.Background(), "s1"); err != nil {
		t.Fatalf("clear goal: %v", err)
	}
	ev = <-events
	if ev.Kind != eventbus.KindGoalCleared {
		t.Fatalf("clear event = %#v", ev)
	}
}

func TestAppWiresLSPManager(t *testing.T) {
	app, err := New(context.Background(), Config{
		Workspace:   filepath.Join(t.TempDir(), "workspace"),
		DataDir:     t.TempDir(),
		Provider:    "mock",
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("new app: %v", err)
	}
	defer app.Close(context.Background())
	if app.LSP == nil {
		t.Fatal("lsp manager should be wired")
	}
}

func TestAppExposesMemoryToolsOnlyWhenEnabled(t *testing.T) {
	enabled, err := New(context.Background(), Config{
		Workspace:   filepath.Join(t.TempDir(), "workspace"),
		DataDir:     t.TempDir(),
		Provider:    "mock",
		AutoApprove: true,
		Memories:    memory.Config{Enabled: true},
	})
	if err != nil {
		t.Fatalf("new enabled app: %v", err)
	}
	defer enabled.Close(context.Background())
	if !appHasTool(enabled, "memory_save") || !appHasTool(enabled, "memory_find") {
		t.Fatalf("enabled specs = %#v", enabled.Tools.Specs())
	}
	if enabled.Memory == nil {
		t.Fatal("enabled app should have memory service")
	}

	disabled, err := New(context.Background(), Config{
		Workspace:   filepath.Join(t.TempDir(), "workspace"),
		DataDir:     t.TempDir(),
		Provider:    "mock",
		AutoApprove: true,
		Memories:    memory.Config{Enabled: false, EmbeddingModel: memory.DefaultEmbeddingModel},
	})
	if err != nil {
		t.Fatalf("new disabled app: %v", err)
	}
	defer disabled.Close(context.Background())
	if appHasTool(disabled, "memory_save") || appHasTool(disabled, "memory_find") {
		t.Fatalf("disabled specs = %#v", disabled.Tools.Specs())
	}
}

func TestAppSetMemoriesEnabledRebuildsTools(t *testing.T) {
	app, err := New(context.Background(), Config{
		Workspace:   filepath.Join(t.TempDir(), "workspace"),
		DataDir:     t.TempDir(),
		Provider:    "mock",
		AutoApprove: true,
		Memories:    memory.Config{Enabled: false, EmbeddingModel: memory.DefaultEmbeddingModel},
	})
	if err != nil {
		t.Fatalf("new app: %v", err)
	}
	defer app.Close(context.Background())
	if appHasTool(app, "memory_save") {
		t.Fatalf("memory_save should start disabled: %#v", app.Tools.Specs())
	}
	if err := app.SetMemoriesEnabled(true); err != nil {
		t.Fatalf("enable memories: %v", err)
	}
	if !app.Config.Memories.Enabled || app.Memory == nil || !appHasTool(app, "memory_save") {
		t.Fatalf("enabled config=%#v memory=%v specs=%#v", app.Config.Memories, app.Memory, app.Tools.Specs())
	}
	if err := app.SetMemoriesEnabled(false); err != nil {
		t.Fatalf("disable memories: %v", err)
	}
	if app.Config.Memories.Enabled || appHasTool(app, "memory_save") {
		t.Fatalf("disabled config=%#v specs=%#v", app.Config.Memories, app.Tools.Specs())
	}
}

func TestAppMemoryIntegrationSaveQueryRecallAndDisable(t *testing.T) {
	useTestMemoryEmbedder(t, map[string][]float32{
		"prefer event bus plugins": {1, 0, 0},
		"event bus":                {1, 0, 0},
	})
	ctx := context.Background()
	app, err := New(ctx, Config{
		Workspace:   filepath.Join(t.TempDir(), "workspace"),
		DataDir:     t.TempDir(),
		Provider:    "mock",
		Model:       "mock",
		Reasoning:   "none",
		AutoApprove: true,
		Memories:    memory.Config{Enabled: true, AutoRecall: true, EmbeddingModel: memory.DefaultEmbeddingModel},
	})
	if err != nil {
		t.Fatalf("new app: %v", err)
	}
	defer app.Close(ctx)

	saved, err := app.Tools.Run(ctx, tools.Call{Name: "memory_save", Input: map[string]any{"content": "prefer event bus plugins"}})
	if err != nil {
		t.Fatalf("memory_save: %v", err)
	}
	savedData, ok := saved.Data.(memory.ToolEntry)
	if saved.Error != "" || !ok || savedData.Content != "prefer event bus plugins" {
		t.Fatalf("save result = %#v", saved)
	}
	found, err := app.Tools.Run(ctx, tools.Call{Name: "memory_find", Input: map[string]any{"query": "event bus", "limit": 1}})
	if err != nil {
		t.Fatalf("memory_find: %v", err)
	}
	if found.Error != "" || !strings.Contains(found.Text, "prefer event bus plugins") {
		t.Fatalf("find result = %#v", found)
	}

	events := app.Bus.Subscribe(ctx, eventbus.Filter{Kinds: []eventbus.Kind{eventbus.KindMemoryRecalled}})
	if _, err := app.RunPrompt(ctx, "event bus"); err != nil {
		t.Fatalf("run prompt: %v", err)
	}
	select {
	case ev := <-events:
		if ev.Kind != eventbus.KindMemoryRecalled {
			t.Fatalf("memory event = %#v", ev)
		}
	default:
		t.Fatal("expected prompt-time memory recall event")
	}

	if err := app.SetMemoriesEnabled(false); err != nil {
		t.Fatalf("disable memories: %v", err)
	}
	if appHasTool(app, "memory_save") || appHasTool(app, "memory_find") {
		t.Fatalf("memory tools should be absent after disable: %#v", app.Tools.Specs())
	}
}

func TestAppMemoryScopesSharedDataDirByWorkspace(t *testing.T) {
	useTestMemoryEmbedder(t, map[string][]float32{
		"local workspace memory": {1, 0, 0},
		"local":                  {1, 0, 0},
	})
	ctx := context.Background()
	dataDir := t.TempDir()
	first, err := New(ctx, Config{
		Workspace:   filepath.Join(t.TempDir(), "repo-a"),
		DataDir:     dataDir,
		Provider:    "mock",
		AutoApprove: true,
		Memories:    memory.Config{Enabled: true, EmbeddingModel: memory.DefaultEmbeddingModel},
	})
	if err != nil {
		t.Fatalf("new first app: %v", err)
	}
	defer first.Close(ctx)
	second, err := New(ctx, Config{
		Workspace:   filepath.Join(t.TempDir(), "repo-b"),
		DataDir:     dataDir,
		Provider:    "mock",
		AutoApprove: true,
		Memories:    memory.Config{Enabled: true, EmbeddingModel: memory.DefaultEmbeddingModel},
	})
	if err != nil {
		t.Fatalf("new second app: %v", err)
	}
	defer second.Close(ctx)

	if _, err := first.Memory.Save(ctx, "local workspace memory", "test"); err != nil {
		t.Fatalf("save first memory: %v", err)
	}
	firstResults, err := first.Memory.Query(ctx, "local", 5)
	if err != nil {
		t.Fatalf("query first: %v", err)
	}
	if len(firstResults) != 1 {
		t.Fatalf("first results = %#v", firstResults)
	}
	secondResults, err := second.Memory.Query(ctx, "local", 5)
	if err != nil {
		t.Fatalf("query second: %v", err)
	}
	if len(secondResults) != 0 {
		t.Fatalf("second workspace saw first memory: %#v", secondResults)
	}
}

func TestNewAppLoadsRepoContextMessages(t *testing.T) {
	workspace := filepath.Join(t.TempDir(), "workspace")
	if err := os.MkdirAll(workspace, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(workspace, "AGENTS.md"), []byte("repo rules"), 0o644); err != nil {
		t.Fatalf("write agents: %v", err)
	}
	if err := os.WriteFile(filepath.Join(workspace, ".gode.toml"), []byte(`[agent]
extra_context = "inline repo context"
`), 0o644); err != nil {
		t.Fatalf("write config: %v", err)
	}

	app, err := New(context.Background(), Config{
		Workspace:   workspace,
		DataDir:     t.TempDir(),
		HomeDir:     t.TempDir(),
		Provider:    "mock",
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("new app: %v", err)
	}
	defer app.Close(context.Background())

	joined := ""
	for _, msg := range app.contextMessages {
		joined += msg.Content + "\n"
	}
	for _, want := range []string{"repo rules", "AGENTS.md", "inline repo context"} {
		if !strings.Contains(joined, want) {
			t.Fatalf("context messages missing %q:\n%s", want, joined)
		}
	}
}

func TestNewAppDiscoversProjectSkills(t *testing.T) {
	workspace := filepath.Join(t.TempDir(), "workspace")
	skillDir := filepath.Join(workspace, ".agents", "skills", "go-tests")
	if err := os.MkdirAll(skillDir, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(skillDir, "SKILL.md"), []byte(`---
name: go-tests
description: Run tests
---
Run Go tests.
`), 0o644); err != nil {
		t.Fatalf("write skill: %v", err)
	}
	app, err := New(context.Background(), Config{
		Workspace:   workspace,
		DataDir:     t.TempDir(),
		HomeDir:     t.TempDir(),
		Provider:    "mock",
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("new app: %v", err)
	}
	defer app.Close(context.Background())

	if len(app.skills) != 1 || app.skills[0].Name != "go-tests" {
		t.Fatalf("skills = %#v", app.skills)
	}
}

func TestNewAppWiresSkillManager(t *testing.T) {
	workspace := filepath.Join(t.TempDir(), "workspace")
	skillDir := filepath.Join(workspace, ".agents", "skills", "go-tests")
	if err := os.MkdirAll(skillDir, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(skillDir, "SKILL.md"), []byte(`---
name: go-tests
description: Run tests
---
Run Go tests.
`), 0o644); err != nil {
		t.Fatalf("write skill: %v", err)
	}
	dataDir := t.TempDir()
	app, err := New(context.Background(), Config{
		Workspace:   workspace,
		DataDir:     dataDir,
		HomeDir:     t.TempDir(),
		Provider:    "mock",
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("new app: %v", err)
	}
	defer app.Close(context.Background())
	if app.SkillManager == nil {
		t.Fatal("skill manager should be wired")
	}
	items, err := app.SkillManager.List(context.Background())
	if err != nil {
		t.Fatalf("list skills: %v", err)
	}
	if len(items) != 1 || items[0].Name != "go-tests" || items[0].State != "enabled" {
		t.Fatalf("items = %#v", items)
	}
	if err := app.SkillManager.SetEnabled(context.Background(), "go-tests", false); err != nil {
		t.Fatalf("disable skill: %v", err)
	}
	settings, err := LoadSettings(dataDir)
	if err != nil {
		t.Fatalf("load settings: %v", err)
	}
	if godeskills.IsSkillEnabled(settings.Skills, godeskills.Skill{Name: "go-tests", Path: filepath.Join(workspace, ".agents", "skills", "go-tests", "SKILL.md")}) {
		t.Fatalf("skills config = %#v", settings.Skills)
	}
}

func TestNewAppLoadsProjectCommands(t *testing.T) {
	workspace := filepath.Join(t.TempDir(), "workspace")
	commandPath := filepath.Join(workspace, ".gode", "commands", "test.md")
	if err := os.MkdirAll(filepath.Dir(commandPath), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(commandPath, []byte("Run tests"), 0o644); err != nil {
		t.Fatalf("write command: %v", err)
	}
	app, err := New(context.Background(), Config{
		Workspace:   workspace,
		DataDir:     t.TempDir(),
		Provider:    "mock",
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("new app: %v", err)
	}
	defer app.Close(context.Background())

	if len(app.commands) != 1 || app.commands[0].ID != "project:test" {
		t.Fatalf("commands = %#v", app.commands)
	}
}

func TestCompactSessionPersistsRawCompactionItems(t *testing.T) {
	dataDir := t.TempDir()
	sessionStore, err := session.Open(dataDir)
	if err != nil {
		t.Fatalf("session store: %v", err)
	}
	if _, err := sessionStore.Ensure(context.Background(), session.Session{ID: "s1", Title: "hello"}); err != nil {
		t.Fatalf("ensure session: %v", err)
	}
	messageStore := messagestore.Open(dataDir)
	if _, err := messageStore.Append(context.Background(), messagestore.Message{SessionID: "s1", Role: messagestore.RoleUser, Text: "hello"}); err != nil {
		t.Fatalf("append message: %v", err)
	}
	journalStore, err := journal.Open(filepath.Join(dataDir, "events.jsonl"))
	if err != nil {
		t.Fatalf("journal: %v", err)
	}
	defer journalStore.Close()
	compactor := &fakeCompactor{
		output: []json.RawMessage{
			json.RawMessage(`{"type":"message","role":"assistant","content":[{"type":"output_text","text":"summary"}]}`),
			json.RawMessage(`{"type":"compaction","encrypted_content":"opaque","id":"cmp_123"}`),
		},
	}
	app := &App{
		Config:   Config{Model: "gpt-5.5"},
		Bus:      eventbus.New(),
		Journal:  journalStore,
		Sessions: sessionStore,
		Messages: messageStore,
		provider: compactor,
	}
	defer app.Bus.Close()

	result, err := app.CompactSession(context.Background(), "s1")
	if err != nil {
		t.Fatalf("compact: %v", err)
	}
	if result.ResponseID != "cmp_resp" || result.OutputItems != 2 {
		t.Fatalf("result = %#v", result)
	}
	if len(compactor.request.Messages) != 1 || compactor.request.Messages[0].Content != "hello" {
		t.Fatalf("compact request messages = %#v", compactor.request.Messages)
	}
	messages, err := messageStore.ListBySession(context.Background(), "s1")
	if err != nil {
		t.Fatalf("list messages: %v", err)
	}
	if len(messages) != 3 {
		t.Fatalf("messages = %#v", messages)
	}
	last := messages[len(messages)-1]
	if last.Role != messagestore.RoleCompaction || last.SourceKind != "compacted" || !strings.Contains(string(last.RawJSON), `"encrypted_content":"opaque"`) {
		t.Fatalf("last message = %#v", last)
	}
	next := providerMessagesFromStored(messages)
	if len(next) != 2 || len(next[0].RawJSON) == 0 || len(next[1].RawJSON) == 0 {
		t.Fatalf("provider messages should preserve raw compaction item: %#v", next)
	}
	events, err := journalStore.Replay(context.Background(), journal.ReplayFilter{SessionID: "s1"})
	if err != nil {
		t.Fatalf("replay: %v", err)
	}
	kinds := map[eventbus.Kind]bool{}
	for _, ev := range events {
		kinds[ev.Kind] = true
	}
	if !kinds[eventbus.KindContextCompactionStarted] || !kinds[eventbus.KindContextCompactionCompleted] {
		t.Fatalf("missing compaction events: %#v", kinds)
	}
}

func TestCompactSessionFailureLeavesMessagesUntouched(t *testing.T) {
	dataDir := t.TempDir()
	sessionStore, err := session.Open(dataDir)
	if err != nil {
		t.Fatalf("session store: %v", err)
	}
	if _, err := sessionStore.Ensure(context.Background(), session.Session{ID: "s1", Title: "hello"}); err != nil {
		t.Fatalf("ensure session: %v", err)
	}
	messageStore := messagestore.Open(dataDir)
	if _, err := messageStore.Append(context.Background(), messagestore.Message{SessionID: "s1", Role: messagestore.RoleUser, Text: "hello"}); err != nil {
		t.Fatalf("append message: %v", err)
	}
	journalStore, err := journal.Open(filepath.Join(dataDir, "events.jsonl"))
	if err != nil {
		t.Fatalf("journal: %v", err)
	}
	defer journalStore.Close()
	app := &App{
		Config:   Config{Model: "gpt-5.5"},
		Bus:      eventbus.New(),
		Journal:  journalStore,
		Sessions: sessionStore,
		Messages: messageStore,
		provider: &fakeCompactor{err: errors.New("compact failed")},
	}
	defer app.Bus.Close()

	if _, err := app.CompactSession(context.Background(), "s1"); err == nil {
		t.Fatal("expected compact error")
	}
	messages, err := messageStore.ListBySession(context.Background(), "s1")
	if err != nil {
		t.Fatalf("list messages: %v", err)
	}
	if len(messages) != 1 || messages[0].Text != "hello" {
		t.Fatalf("messages should be untouched, got %#v", messages)
	}
	events, err := journalStore.Replay(context.Background(), journal.ReplayFilter{SessionID: "s1", Kinds: []eventbus.Kind{eventbus.KindContextCompactionFailed}})
	if err != nil {
		t.Fatalf("replay: %v", err)
	}
	if len(events) != 1 {
		t.Fatalf("failed events = %d", len(events))
	}
}

type fakeCompactor struct {
	request provider.CompactRequest
	output  []json.RawMessage
	err     error
}

func (f *fakeCompactor) Name() string {
	return "openai"
}

func (f *fakeCompactor) Stream(context.Context, provider.Request) (<-chan provider.Event, <-chan error) {
	events := make(chan provider.Event)
	errs := make(chan error)
	close(events)
	close(errs)
	return events, errs
}

func (f *fakeCompactor) Compact(_ context.Context, req provider.CompactRequest) (provider.CompactResult, error) {
	f.request = req
	if f.err != nil {
		return provider.CompactResult{}, f.err
	}
	return provider.CompactResult{ID: "cmp_resp", Output: f.output}, nil
}

func appHasTool(app *App, name string) bool {
	for _, spec := range app.Tools.Specs() {
		if spec.Name == name {
			return true
		}
	}
	return false
}

func useTestMemoryEmbedder(t *testing.T, values map[string][]float32) {
	t.Helper()
	previous := memoryEmbedderFactory
	memoryEmbedderFactory = func(model string) memory.Embedder {
		return appTestMemoryEmbedder{model: model, values: values}
	}
	t.Cleanup(func() {
		memoryEmbedderFactory = previous
	})
}

type appTestMemoryEmbedder struct {
	model  string
	values map[string][]float32
}

func (e appTestMemoryEmbedder) Model() string {
	if strings.TrimSpace(e.model) == "" {
		return memory.DefaultEmbeddingModel
	}
	return e.model
}

func (e appTestMemoryEmbedder) Embed(_ context.Context, input string) (memory.Vector, error) {
	values := e.values[strings.TrimSpace(input)]
	if len(values) == 0 {
		values = []float32{1, 0, 0}
	}
	return memory.Vector{Model: e.Model(), Dimensions: len(values), Values: append([]float32(nil), values...)}, nil
}
