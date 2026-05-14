package main

import (
	"context"
	"path/filepath"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/appserver"
	"github.com/pandelisz/gode/internal/godex/memory"
)

func TestHeadlessRunAndAppServerRespectPersistedMemoryConfig(t *testing.T) {
	root := t.TempDir()
	t.Setenv("HOME", root)
	workspace := filepath.Join(root, "workspace")
	dataDir := filepath.Join(root, ".gode")
	if err := godex.SaveSettings(dataDir, godex.Settings{
		Memories: memory.Settings{
			Enabled:    boolPtr(false),
			AutoRecall: boolPtr(false),
		},
	}); err != nil {
		t.Fatalf("save settings: %v", err)
	}

	runCfg, err := parseConfigWithName("gode run", memoryIntegrationArgs(workspace, dataDir))
	if err != nil {
		t.Fatalf("parse run config: %v", err)
	}
	if runCfg.Memories.Enabled || runCfg.Memories.AutoRecall {
		t.Fatalf("run memories = %#v", runCfg.Memories)
	}
	assertMemoryToolsAbsent(t, runCfg)
	out := captureStdout(t, func() error {
		return runPrompt(context.Background(), append(memoryIntegrationArgs(workspace, dataDir), "hello"))
	})
	if !strings.Contains(out, "mock response") {
		t.Fatalf("run output = %q", out)
	}

	appServerCfg, listen, err := parseAppServerConfig(append([]string{"--listen", "off"}, memoryIntegrationArgs(workspace, dataDir)...))
	if err != nil {
		t.Fatalf("parse app-server config: %v", err)
	}
	if listen.Kind != appserver.TransportOff {
		t.Fatalf("listen kind = %v", listen.Kind)
	}
	if appServerCfg.Memories.Enabled || appServerCfg.Memories.AutoRecall {
		t.Fatalf("app-server memories = %#v", appServerCfg.Memories)
	}
	assertMemoryToolsAbsent(t, appServerCfg)
}

func memoryIntegrationArgs(workspace string, dataDir string) []string {
	return []string{
		"--workspace", workspace,
		"--data-dir", dataDir,
		"--provider", "mock",
		"--model", "mock",
		"--reasoning", "none",
	}
}

func assertMemoryToolsAbsent(t *testing.T, cfg godex.Config) {
	t.Helper()
	app, err := godex.New(context.Background(), cfg)
	if err != nil {
		t.Fatalf("new app: %v", err)
	}
	defer app.Close(context.Background())
	for _, spec := range app.Tools.Specs() {
		switch spec.Name {
		case "memory_save", "memory_find", "memory_read", "memory_update", "memory_delete":
			t.Fatalf("memory tool %q should be absent when disabled: %#v", spec.Name, app.Tools.Specs())
		}
	}
}
