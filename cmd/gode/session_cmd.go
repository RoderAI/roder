package main

import (
	"context"
	"encoding/json"
	"fmt"
	"path/filepath"
	"strings"

	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/journal"
	messagestore "github.com/pandelisz/gode/internal/godex/message"
	"github.com/pandelisz/gode/internal/godex/session"
)

func runSession(args []string) error {
	if len(args) == 0 {
		return fmt.Errorf("usage: gode session list|show|debug|last|rename|delete")
	}
	command := args[0]
	flags := newFlagSet("gode session " + command)
	cfg := godex.DefaultConfig()
	runID := ""
	debugSessionID := ""
	if command == "debug" {
		flags.StringVar(&runID, "run", runID, "run id to filter")
	}
	bindConfigFlags(flags, &cfg)
	switch command {
	case "list", "last":
		if err := flags.Parse(args[1:]); err != nil {
			return err
		}
	case "show", "delete":
		if err := flags.Parse(args[1:]); err != nil {
			return err
		}
		if flags.NArg() != 1 {
			return fmt.Errorf("usage: gode session %s <id>", command)
		}
	case "debug":
		if err := flags.Parse(args[1:]); err != nil {
			return err
		}
		id, parsedRunID, err := parseSessionDebugArgs(flags.Args(), runID)
		if err != nil {
			return err
		}
		debugSessionID = id
		runID = parsedRunID
	case "rename":
		if err := flags.Parse(args[1:]); err != nil {
			return err
		}
		if flags.NArg() < 2 {
			return fmt.Errorf("usage: gode session rename <id> <title>")
		}
	default:
		return fmt.Errorf("unknown session command %q", command)
	}
	loaded, err := loadConfigFromFlags(cfg, flags)
	if err != nil {
		return err
	}
	store, err := session.Open(loaded.Config.DataDir)
	if err != nil {
		return err
	}
	messages := messagestore.Open(loaded.Config.DataDir)
	ctx := context.Background()

	switch command {
	case "list":
		sessions, err := store.List(ctx)
		if err != nil {
			return err
		}
		for _, item := range sessions {
			printSession(item)
		}
	case "last":
		item, ok, err := store.Last(ctx)
		if err != nil {
			return err
		}
		if !ok {
			return nil
		}
		printSession(item)
	case "show":
		id := flags.Arg(0)
		item, ok, err := store.Get(ctx, id)
		if err != nil {
			return err
		}
		if !ok {
			return session.ErrNotFound
		}
		printSession(item)
		list, err := messages.ListBySession(ctx, id)
		if err != nil {
			return err
		}
		for _, msg := range list {
			printMessage(msg)
		}
	case "debug":
		id := debugSessionID
		item, ok, err := store.Get(ctx, id)
		if err != nil {
			return err
		}
		if !ok {
			return session.ErrNotFound
		}
		events, err := readSessionEvents(ctx, loaded.Config.DataDir, id, runID)
		if err != nil {
			return err
		}
		printSession(item)
		for _, ev := range events {
			printEvent(ev)
		}
	case "rename":
		id := flags.Arg(0)
		title := strings.TrimSpace(strings.Join(flags.Args()[1:], " "))
		item, err := store.Rename(ctx, id, title)
		if err != nil {
			return err
		}
		printSession(item)
	case "delete":
		if err := store.Delete(ctx, flags.Arg(0)); err != nil {
			return err
		}
		fmt.Println("deleted\t" + flags.Arg(0))
	}
	return nil
}

func printSession(item session.Session) {
	fmt.Printf("%s\t%s\t%d\t%s\n", item.ID, item.UpdatedAt.Format("2006-01-02T15:04:05Z07:00"), item.MessageCount, item.Title)
}

func printMessage(msg messagestore.Message) {
	if msg.ToolName != "" {
		fmt.Printf("%s\t%s\t%s\n", msg.Role, msg.ToolName, oneLine(msg.Text))
		return
	}
	fmt.Printf("%s\t%s\n", msg.Role, oneLine(msg.Text))
}

func parseSessionDebugArgs(args []string, runID string) (string, string, error) {
	sessionID := ""
	for i := 0; i < len(args); i++ {
		arg := args[i]
		switch {
		case arg == "--run":
			if i+1 >= len(args) {
				return "", "", fmt.Errorf("usage: gode session debug <id> [--run <run_id>]")
			}
			runID = args[i+1]
			i++
		case strings.HasPrefix(arg, "--run="):
			runID = strings.TrimPrefix(arg, "--run=")
		case strings.HasPrefix(arg, "-"):
			return "", "", fmt.Errorf("unknown debug flag %q", arg)
		case sessionID == "":
			sessionID = arg
		default:
			return "", "", fmt.Errorf("usage: gode session debug <id> [--run <run_id>]")
		}
	}
	if sessionID == "" {
		return "", "", fmt.Errorf("usage: gode session debug <id> [--run <run_id>]")
	}
	return sessionID, runID, nil
}

func readSessionEvents(ctx context.Context, dataDir string, sessionID string, runID string) ([]eventbus.Event, error) {
	store, err := journal.Open(filepath.Join(dataDir, "events.jsonl"))
	if err != nil {
		return nil, err
	}
	defer store.Close()
	return store.Replay(ctx, journal.ReplayFilter{SessionID: sessionID, RunID: runID})
}

func printEvent(ev eventbus.Event) {
	payload := ""
	if ev.Payload != nil {
		data, err := json.Marshal(ev.Payload)
		if err == nil {
			payload = string(data)
		}
	}
	parts := []string{ev.Time.Format("2006-01-02T15:04:05Z07:00"), string(ev.Kind), string(ev.Source)}
	if ev.RunID != "" {
		parts = append(parts, ev.RunID)
	}
	if payload != "" {
		parts = append(parts, payload)
	}
	fmt.Println(strings.Join(parts, "\t"))
}

func oneLine(text string) string {
	return strings.Join(strings.Fields(text), " ")
}
