package main

import (
	"context"
	"fmt"
	"strings"

	"github.com/pandelisz/gode/internal/godex"
	messagestore "github.com/pandelisz/gode/internal/godex/message"
	"github.com/pandelisz/gode/internal/godex/session"
)

func runSession(args []string) error {
	if len(args) == 0 {
		return fmt.Errorf("usage: gode session list|show|last|rename|delete")
	}
	command := args[0]
	flags := newFlagSet("gode session " + command)
	cfg := godex.DefaultConfig()
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

func oneLine(text string) string {
	return strings.Join(strings.Fields(text), " ")
}
