package main

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"strings"

	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/goals"
	tuigoals "github.com/pandelisz/gode/internal/tui/goals"
)

func runGoal(ctx context.Context, args []string) error {
	if len(args) == 0 {
		return fmt.Errorf("usage: gode goal get|set|pause|resume|clear --session <id>")
	}
	switch args[0] {
	case "get":
		return runGoalGet(ctx, args[1:])
	case "set":
		return runGoalSet(ctx, args[1:])
	case "pause":
		return runGoalStatus(ctx, args[1:], goals.StatusPaused)
	case "resume":
		return runGoalStatus(ctx, args[1:], goals.StatusActive)
	case "clear":
		return runGoalClear(ctx, args[1:])
	default:
		return fmt.Errorf("unknown goal command %q", args[0])
	}
}

func runGoalGet(ctx context.Context, args []string) error {
	cfg, sessionID, jsonOut, err := parseGoalCommon("gode goal get", args)
	if err != nil {
		return err
	}
	app, err := godex.New(ctx, cfg)
	if err != nil {
		return err
	}
	defer app.Close(ctx)
	goal, err := app.GetGoal(ctx, sessionID)
	if err != nil {
		return err
	}
	return printGoal(goal, jsonOut)
}

func runGoalSet(ctx context.Context, args []string) error {
	flags := newFlagSet("gode goal set")
	cfg := godex.DefaultConfig()
	sessionID := ""
	objective := ""
	tokenBudget := int64(0)
	jsonOut := false
	flags.StringVar(&sessionID, "session", sessionID, "session id")
	flags.StringVar(&objective, "objective", objective, "goal objective")
	flags.Int64Var(&tokenBudget, "token-budget", tokenBudget, "positive token budget")
	flags.BoolVar(&jsonOut, "json", jsonOut, "print JSON")
	bindConfigFlags(flags, &cfg)
	if err := flags.Parse(args); err != nil {
		return err
	}
	loaded, err := loadConfigFromFlags(cfg, flags)
	if err != nil {
		return err
	}
	sessionID = strings.TrimSpace(sessionID)
	if sessionID == "" {
		return fmt.Errorf("--session is required")
	}
	if strings.TrimSpace(objective) == "" {
		return fmt.Errorf("--objective is required")
	}
	var budget *int64
	if tokenBudget > 0 {
		budget = &tokenBudget
	} else if tokenBudget < 0 {
		return goals.ErrInvalidBudget
	}
	app, err := godex.New(ctx, loaded.Config)
	if err != nil {
		return err
	}
	defer app.Close(ctx)
	goal, err := app.SetGoal(ctx, goals.SetRequest{SessionID: sessionID, Objective: objective, TokenBudget: budget, ReplaceExisting: true})
	if err != nil {
		return err
	}
	return printGoal(goal, jsonOut)
}

func runGoalStatus(ctx context.Context, args []string, status goals.GoalStatus) error {
	cfg, sessionID, jsonOut, err := parseGoalCommon("gode goal "+string(status), args)
	if err != nil {
		return err
	}
	app, err := godex.New(ctx, cfg)
	if err != nil {
		return err
	}
	defer app.Close(ctx)
	goal, err := app.SetGoal(ctx, goals.SetRequest{SessionID: sessionID, Status: status})
	if err != nil {
		return err
	}
	return printGoal(goal, jsonOut)
}

func runGoalClear(ctx context.Context, args []string) error {
	cfg, sessionID, jsonOut, err := parseGoalCommon("gode goal clear", args)
	if err != nil {
		return err
	}
	app, err := godex.New(ctx, cfg)
	if err != nil {
		return err
	}
	defer app.Close(ctx)
	if err := app.ClearGoal(ctx, sessionID); err != nil {
		return err
	}
	if jsonOut {
		return json.NewEncoder(os.Stdout).Encode(map[string]any{"cleared": true, "session_id": sessionID})
	}
	fmt.Printf("cleared\t%s\n", sessionID)
	return nil
}

func parseGoalCommon(name string, args []string) (godex.Config, string, bool, error) {
	flags := newFlagSet(name)
	cfg := godex.DefaultConfig()
	sessionID := ""
	jsonOut := false
	flags.StringVar(&sessionID, "session", sessionID, "session id")
	flags.BoolVar(&jsonOut, "json", jsonOut, "print JSON")
	bindConfigFlags(flags, &cfg)
	if err := flags.Parse(args); err != nil {
		return godex.Config{}, "", false, err
	}
	loaded, err := loadConfigFromFlags(cfg, flags)
	if err != nil {
		return godex.Config{}, "", false, err
	}
	sessionID = strings.TrimSpace(sessionID)
	if sessionID == "" {
		return godex.Config{}, "", false, fmt.Errorf("--session is required")
	}
	return loaded.Config, sessionID, jsonOut, nil
}

func printGoal(goal *goals.Goal, jsonOut bool) error {
	if jsonOut {
		return json.NewEncoder(os.Stdout).Encode(map[string]any{"goal": goal})
	}
	if goal == nil {
		fmt.Println("goal\tnone")
		return nil
	}
	fmt.Printf("goal\t%s\t%s\n", goal.Status, goal.Objective)
	fmt.Printf("elapsed\t%s\n", tuigoals.Elapsed(goal.TimeUsedSeconds))
	if budget := tuigoals.Budget(goal.TokensUsed, goal.TokenBudget); budget != "" {
		fmt.Printf("budget\t%s\n", budget)
	}
	return nil
}
