package roadmap

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
)

type State struct {
	FocusedDocument string            `json:"focusedDocument,omitempty"`
	FocusedTaskID   string            `json:"focusedTaskId,omitempty"`
	AttachedThreads map[string]string `json:"attachedThreads,omitempty"`
}

func LoadState(dataDir string) (State, error) {
	path := statePath(dataDir)
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return State{}, nil
		}
		return State{}, fmt.Errorf("read roadmap state: %w", err)
	}
	var state State
	if err := json.Unmarshal(data, &state); err != nil {
		return State{}, fmt.Errorf("parse roadmap state: %w", err)
	}
	return state, nil
}

func SaveState(dataDir string, state State) error {
	path := statePath(dataDir)
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return fmt.Errorf("create roadmap state dir: %w", err)
	}
	data, err := json.MarshalIndent(state, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal roadmap state: %w", err)
	}
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, append(data, '\n'), 0o600); err != nil {
		return fmt.Errorf("write roadmap state: %w", err)
	}
	if err := os.Rename(tmp, path); err != nil {
		return fmt.Errorf("commit roadmap state: %w", err)
	}
	return nil
}

func SetTaskChecked(path string, taskID string, checked bool, evidence string) error {
	doc, err := ParseFile(path)
	if err != nil {
		return err
	}
	lineIndex := -1
	for _, task := range doc.Tasks {
		if task.ID == taskID {
			lineIndex = task.Line - 1
			break
		}
	}
	if lineIndex < 0 {
		return fmt.Errorf("task %s not found", taskID)
	}
	line := doc.Lines[lineIndex]
	replacement := " "
	if checked {
		replacement = "x"
	}
	doc.Lines[lineIndex] = checkboxLine.ReplaceAllString(line, "${1}"+replacement+"${3}${4}")
	if doc.Lines[lineIndex] == line {
		return fmt.Errorf("task %s line is not a checkbox", taskID)
	}
	output := strings.Join(doc.Lines, "\n")
	if strings.HasSuffix(doc.Raw, "\n") {
		output += "\n"
	}
	return os.WriteFile(path, []byte(output), 0o644)
}

func ListDocuments(workspace string, includeIndex bool) ([]string, error) {
	matches, err := filepath.Glob(filepath.Join(workspace, "roadmap", "*.md"))
	if err != nil {
		return nil, err
	}
	docs := make([]string, 0, len(matches))
	for _, path := range matches {
		if !includeIndex && strings.HasPrefix(filepath.Base(path), "00-") {
			continue
		}
		docs = append(docs, path)
	}
	sort.Slice(docs, func(i, j int) bool {
		left, right := phaseNumber(docs[i]), phaseNumber(docs[j])
		if left != right {
			return left < right
		}
		return docs[i] < docs[j]
	})
	return docs, nil
}

func statePath(dataDir string) string {
	return filepath.Join(dataDir, "roadmaps", "state.json")
}

func phaseNumber(path string) int {
	name := filepath.Base(path)
	prefix, _, ok := strings.Cut(name, "-")
	if !ok {
		return 1_000_000
	}
	n, err := strconv.Atoi(prefix)
	if err != nil {
		return 1_000_000
	}
	return n
}
