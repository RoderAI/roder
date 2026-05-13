package session

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/pandelisz/gode/internal/godex/journal"
)

type RepairReport struct {
	Sessions      int
	Turns         int
	Items         int
	MissingIndex  int
	MissingTurns  int
	InvalidItems  int
	RepairActions []string
	Diagnostics   []string
}

func Doctor(ctx context.Context, dataDir string) (RepairReport, error) {
	report := RepairReport{}
	indexSessions, err := readIndexSessions(dataDir)
	if err != nil {
		report.Diagnostics = append(report.Diagnostics, err.Error())
	}
	report.Sessions = len(indexSessions)
	indexIDs := map[string]bool{}
	for _, stored := range indexSessions {
		indexIDs[stored.ID] = true
	}

	sessionIDs, err := sessionDirs(dataDir)
	if err != nil {
		return report, err
	}
	turnStore, err := OpenTurnStore(dataDir)
	if err != nil {
		return report, err
	}
	itemStore, err := OpenItemStore(dataDir)
	if err != nil {
		return report, err
	}
	for _, sessionID := range sessionIDs {
		if err := ctx.Err(); err != nil {
			return report, err
		}
		if !indexIDs[sessionID] {
			report.MissingIndex++
			report.Diagnostics = append(report.Diagnostics, "missing index entry: "+sessionID)
		}
		turns, err := turnStore.ListBySession(ctx, sessionID)
		if err != nil {
			report.Diagnostics = append(report.Diagnostics, err.Error())
		}
		report.Turns += len(turns)
		turnIDs := map[string]bool{}
		for _, turn := range turns {
			turnIDs[turn.ID] = true
		}
		items, err := itemStore.ListBySession(ctx, sessionID)
		if err != nil {
			report.InvalidItems++
			report.Diagnostics = append(report.Diagnostics, err.Error())
			continue
		}
		report.Items += len(items)
		for _, item := range items {
			if item.TurnID != "" && !turnIDs[item.TurnID] {
				report.MissingTurns++
				report.Diagnostics = append(report.Diagnostics, "missing turn record: "+sessionID+"/"+item.TurnID)
			}
		}
	}
	return report, nil
}

func RepairFromJournal(ctx context.Context, dataDir string, journalPath string) (RepairReport, error) {
	if strings.TrimSpace(journalPath) == "" {
		journalPath = filepath.Join(dataDir, "events.jsonl")
	}
	journalStore, err := journal.Open(journalPath)
	if err != nil {
		return RepairReport{}, err
	}
	defer journalStore.Close()

	repairDir, err := os.MkdirTemp(filepath.Dir(dataDir), ".gode-repair-*")
	if err != nil {
		return RepairReport{}, err
	}
	defer os.RemoveAll(repairDir)
	sessionStore, err := Open(repairDir)
	if err != nil {
		return RepairReport{}, err
	}
	turnStore, err := OpenTurnStore(repairDir)
	if err != nil {
		return RepairReport{}, err
	}
	itemStore, err := OpenItemStore(repairDir)
	if err != nil {
		return RepairReport{}, err
	}
	backfill, err := Backfill(ctx, journalStore, BackfillStores{Sessions: sessionStore, Turns: turnStore, Items: itemStore})
	if err != nil {
		return RepairReport{}, err
	}

	report := RepairReport{Sessions: backfill.Sessions, Turns: backfill.Turns, Items: backfill.Items}
	generatedSessions, _ := sessionDirs(repairDir)
	if err := copyRepairFile(filepath.Join(repairDir, "sessions", indexFileName), filepath.Join(dataDir, "sessions", indexFileName+".repaired"), &report); err != nil {
		return report, err
	}
	for _, sessionID := range generatedSessions {
		srcDir := filepath.Join(repairDir, "sessions", sessionID)
		dstDir := filepath.Join(dataDir, "sessions", sessionID)
		if err := copyRepairFile(filepath.Join(srcDir, turnsFileName), filepath.Join(dstDir, turnsFileName+".repaired"), &report); err != nil {
			return report, err
		}
		if err := copyRepairFile(filepath.Join(srcDir, itemsFileName), filepath.Join(dstDir, itemsFileName+".repaired"), &report); err != nil {
			return report, err
		}
	}
	return report, nil
}

func readIndexSessions(dataDir string) ([]Session, error) {
	data, err := os.ReadFile(filepath.Join(dataDir, "sessions", indexFileName))
	if errors.Is(err, os.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, fmt.Errorf("read session index: %w", err)
	}
	if strings.TrimSpace(string(data)) == "" {
		return nil, nil
	}
	var file indexFile
	if err := json.Unmarshal(data, &file); err != nil {
		return nil, fmt.Errorf("parse session index %s: %w", filepath.Join(dataDir, "sessions", indexFileName), err)
	}
	return file.Sessions, nil
}

func sessionDirs(dataDir string) ([]string, error) {
	entries, err := os.ReadDir(filepath.Join(dataDir, "sessions"))
	if errors.Is(err, os.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	var ids []string
	for _, entry := range entries {
		if entry.IsDir() {
			ids = append(ids, entry.Name())
		}
	}
	sort.Strings(ids)
	return ids, nil
}

func copyRepairFile(src string, dst string, report *RepairReport) error {
	data, err := os.ReadFile(src)
	if errors.Is(err, os.ErrNotExist) {
		return nil
	}
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(dst), 0o700); err != nil {
		return err
	}
	if err := os.WriteFile(dst, data, 0o600); err != nil {
		return err
	}
	report.RepairActions = append(report.RepairActions, "wrote "+dst)
	return nil
}
