package agent

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/provider"
	godeskills "github.com/pandelisz/gode/internal/godex/skills"
	"github.com/pandelisz/gode/internal/godex/tools"
)

func TestRunnerAttachesImplicitSkillAfterScriptTool(t *testing.T) {
	workspace := t.TempDir()
	skillPath := filepath.Join(workspace, ".agents", "skills", "go-dev", "SKILL.md")
	writeAgentSkill(t, skillPath)
	scriptPath := filepath.Join(filepath.Dir(skillPath), "scripts", "check.sh")
	if err := os.MkdirAll(filepath.Dir(scriptPath), 0o700); err != nil {
		t.Fatalf("mkdir script: %v", err)
	}
	if err := os.WriteFile(scriptPath, []byte("#!/bin/sh\n"), 0o600); err != nil {
		t.Fatalf("write script: %v", err)
	}
	skill, err := godeskills.ParseFile(skillPath)
	if err != nil {
		t.Fatalf("parse skill: %v", err)
	}

	reg := tools.NewRegistry(tools.WithAutoApprove(true))
	reg.Register(tools.Tool{
		Name:        "shell",
		Description: "shell",
		ReadOnly:    true,
		Run: func(context.Context, tools.Call) (tools.Result, error) {
			return tools.Result{Text: "checked"}, nil
		},
	})
	script := &scriptedProvider{streams: [][]provider.Event{
		{
			{Kind: provider.EventToolCall, ToolRequest: &provider.ToolRequest{
				ID:        "call_shell",
				Name:      "shell",
				Input:     map[string]any{"command": "bash .agents/skills/go-dev/scripts/check.sh"},
				Arguments: `{"command":"bash .agents/skills/go-dev/scripts/check.sh"}`,
			}},
			{Kind: provider.EventCompleted},
		},
		{{Kind: provider.EventDelta, Text: "done"}, {Kind: provider.EventCompleted, Text: "done"}},
	}}
	runner := NewRunner(Config{
		Bus:       eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Tools:     reg,
		Provider:  script,
		Workspace: workspace,
		Skills:    []godeskills.Skill{skill},
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{Prompt: "check"}); err != nil {
		t.Fatalf("run: %v", err)
	}
	messages := script.requests[1].Messages
	if len(messages) != 5 {
		t.Fatalf("messages = %#v", messages)
	}
	if messages[4].Role != provider.RoleUser || !strings.Contains(messages[4].Content, "Implicit skill context") || !strings.Contains(messages[4].Content, "<name>go-dev</name>") {
		t.Fatalf("implicit skill message = %#v", messages[4])
	}
	if len(script.requests[1].InputItems) == 0 || !strings.Contains(script.requests[1].InputItems[len(script.requests[1].InputItems)-1].Text, "<name>go-dev</name>") {
		t.Fatalf("input items missing implicit skill: %#v", script.requests[1].InputItems)
	}
}

func TestRunnerRespectsImplicitSkillPolicy(t *testing.T) {
	workspace := t.TempDir()
	skillPath := filepath.Join(workspace, ".agents", "skills", "go-dev", "SKILL.md")
	writeAgentSkill(t, skillPath)
	metadataPath := filepath.Join(filepath.Dir(skillPath), "agents", "openai.yaml")
	if err := os.MkdirAll(filepath.Dir(metadataPath), 0o700); err != nil {
		t.Fatalf("mkdir metadata: %v", err)
	}
	if err := os.WriteFile(metadataPath, []byte("policy:\n  allow_implicit_invocation: false\n"), 0o600); err != nil {
		t.Fatalf("write metadata: %v", err)
	}
	skill, err := godeskills.ParseFile(skillPath)
	if err != nil {
		t.Fatalf("parse skill: %v", err)
	}

	reg := tools.NewRegistry(tools.WithAutoApprove(true))
	reg.Register(tools.Tool{
		Name:        "read_file",
		Description: "read",
		ReadOnly:    true,
		Run: func(context.Context, tools.Call) (tools.Result, error) {
			return tools.Result{Text: "read skill"}, nil
		},
	})
	script := &scriptedProvider{streams: [][]provider.Event{
		{
			{Kind: provider.EventToolCall, ToolRequest: &provider.ToolRequest{
				ID:        "call_read",
				Name:      "read_file",
				Input:     map[string]any{"path": skillPath},
				Arguments: `{"path":"` + skillPath + `"}`,
			}},
			{Kind: provider.EventCompleted},
		},
		{{Kind: provider.EventDelta, Text: "done"}, {Kind: provider.EventCompleted, Text: "done"}},
	}}
	runner := NewRunner(Config{
		Bus:       eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Tools:     reg,
		Provider:  script,
		Workspace: workspace,
		Skills:    []godeskills.Skill{skill},
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{Prompt: "read"}); err != nil {
		t.Fatalf("run: %v", err)
	}
	for _, message := range script.requests[1].Messages {
		if strings.Contains(message.Content, "Implicit skill context") {
			t.Fatalf("implicit skill should be disabled: %#v", script.requests[1].Messages)
		}
	}
}

func writeAgentSkill(t *testing.T, path string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		t.Fatalf("mkdir skill: %v", err)
	}
	content := "---\nname: go-dev\ndescription: Go development skill\n---\n\n# Go Development\n\nRun Go tests.\n"
	if err := os.WriteFile(path, []byte(content), 0o600); err != nil {
		t.Fatalf("write skill: %v", err)
	}
}
