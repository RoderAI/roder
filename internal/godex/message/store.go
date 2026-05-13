package message

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/google/uuid"
	"github.com/pandelisz/gode/internal/godex/eventbus"
)

const messagesFileName = "messages.jsonl"

type Store struct {
	mu  sync.Mutex
	dir string
	now func() time.Time
}

func Open(dataDir string) *Store {
	return &Store{
		dir: filepath.Join(dataDir, "sessions"),
		now: func() time.Time {
			return time.Now().UTC()
		},
	}
}

func (s *Store) SessionPath(sessionID string) string {
	if s == nil {
		return ""
	}
	return s.path(sessionID)
}

func (s *Store) Append(ctx context.Context, msg Message) (Message, error) {
	if err := ctx.Err(); err != nil {
		return Message{}, err
	}
	if strings.TrimSpace(msg.SessionID) == "" {
		return Message{}, errors.New("session_id is required")
	}
	if msg.ID == "" {
		msg.ID = uuid.NewString()
	}
	if msg.CreatedAt.IsZero() {
		msg.CreatedAt = s.now()
	}
	data, err := json.Marshal(msg)
	if err != nil {
		return Message{}, err
	}
	path := s.path(msg.SessionID)

	s.mu.Lock()
	defer s.mu.Unlock()
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return Message{}, fmt.Errorf("message dir: %w", err)
	}
	file, err := os.OpenFile(path, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0o600)
	if err != nil {
		return Message{}, fmt.Errorf("open messages: %w", err)
	}
	defer file.Close()
	if _, err := file.Write(append(data, '\n')); err != nil {
		return Message{}, fmt.Errorf("write message: %w", err)
	}
	return msg, nil
}

func (s *Store) AppendProjected(ctx context.Context, ev eventbus.Event) ([]Message, error) {
	projected := ProjectionFromEvent(ev)
	out := make([]Message, 0, len(projected))
	for _, msg := range projected {
		saved, err := s.Append(ctx, msg)
		if err != nil {
			return nil, err
		}
		out = append(out, saved)
	}
	return out, nil
}

func (s *Store) ListBySession(ctx context.Context, sessionID string) ([]Message, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	messages, err := s.readSession(sessionID)
	if err != nil {
		return nil, err
	}
	return coalesceAssistant(messages), nil
}

func (s *Store) ListByRun(ctx context.Context, sessionID string, runID string) ([]Message, error) {
	messages, err := s.ListBySession(ctx, sessionID)
	if err != nil {
		return nil, err
	}
	out := make([]Message, 0, len(messages))
	for _, msg := range messages {
		if msg.RunID == runID {
			out = append(out, msg)
		}
	}
	return out, nil
}

func ProjectionFromEvent(ev eventbus.Event) []Message {
	base := Message{
		ID:         ev.ID,
		SessionID:  ev.SessionID,
		RunID:      ev.RunID,
		SourceKind: string(ev.Kind),
		CreatedAt:  ev.Time,
	}
	switch ev.Kind {
	case eventbus.KindUserPromptSubmitted:
		var payload struct {
			Prompt string `json:"prompt"`
		}
		_ = ev.DecodePayload(&payload)
		return single(base, RoleUser, payload.Prompt)
	case eventbus.KindAssistantDelta, eventbus.KindAssistantCompleted:
		var payload struct {
			Text  string `json:"text"`
			Phase string `json:"phase"`
		}
		_ = ev.DecodePayload(&payload)
		base.Phase = strings.TrimSpace(payload.Phase)
		return single(base, RoleAssistant, payload.Text)
	case eventbus.KindToolRequested:
		var payload struct {
			Tool       string `json:"tool"`
			ToolCallID string `json:"tool_call_id"`
		}
		_ = ev.DecodePayload(&payload)
		base.ToolName = payload.Tool
		base.ToolCallID = payload.ToolCallID
		return single(base, RoleTool, "requested")
	case eventbus.KindToolCompleted:
		var payload struct {
			Tool       string         `json:"tool"`
			ToolCallID string         `json:"tool_call_id"`
			Input      map[string]any `json:"input"`
			Text       string         `json:"text"`
		}
		_ = ev.DecodePayload(&payload)
		base.ToolName = payload.Tool
		base.ToolCallID = payload.ToolCallID
		return single(base, RoleTool, summarizeToolProjection(payload.Tool, payload.Input, payload.Text))
	case eventbus.KindToolFailed:
		var payload struct {
			Tool       string `json:"tool"`
			ToolCallID string `json:"tool_call_id"`
			Error      string `json:"error"`
			Text       string `json:"text"`
		}
		_ = ev.DecodePayload(&payload)
		base.ToolName = payload.Tool
		base.ToolCallID = payload.ToolCallID
		text := payload.Text
		if text == "" {
			text = payload.Error
		}
		return single(base, RoleTool, text)
	case eventbus.KindRunFailed:
		var payload struct {
			Error  string `json:"error"`
			Detail string `json:"detail"`
		}
		_ = ev.DecodePayload(&payload)
		text := payload.Detail
		if text == "" {
			text = payload.Error
		}
		return single(base, RoleError, text)
	default:
		return nil
	}
}

func summarizeToolProjection(tool string, input map[string]any, output string) string {
	switch tool {
	case "read_file":
		path := strings.TrimSpace(inputString(input, "path"))
		if path == "" {
			return "read file"
		}
		return "read " + path
	default:
		return output
	}
}

func inputString(input map[string]any, key string) string {
	if input == nil {
		return ""
	}
	switch value := input[key].(type) {
	case string:
		return value
	case []byte:
		return string(value)
	default:
		return ""
	}
}

func single(base Message, role string, text string) []Message {
	text = strings.TrimSpace(text)
	if base.SessionID == "" || text == "" {
		return nil
	}
	base.Role = role
	base.Text = text
	return []Message{base}
}

func (s *Store) readSession(sessionID string) ([]Message, error) {
	path := s.path(sessionID)
	file, err := os.Open(path)
	if errors.Is(err, os.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, fmt.Errorf("open messages: %w", err)
	}
	defer file.Close()

	var messages []Message
	scanner := bufio.NewScanner(file)
	scanner.Buffer(make([]byte, 64*1024), 8*1024*1024)
	for scanner.Scan() {
		var msg Message
		if err := json.Unmarshal(scanner.Bytes(), &msg); err != nil {
			return nil, fmt.Errorf("parse message: %w", err)
		}
		messages = append(messages, msg)
	}
	if err := scanner.Err(); err != nil {
		return nil, fmt.Errorf("scan messages: %w", err)
	}
	return messages, nil
}

func (s *Store) path(sessionID string) string {
	return filepath.Join(s.dir, sessionID, messagesFileName)
}

func coalesceAssistant(messages []Message) []Message {
	out := make([]Message, 0, len(messages))
	assistantByRun := map[string]int{}
	for _, msg := range messages {
		if msg.Role != RoleAssistant || msg.RunID == "" {
			out = append(out, msg)
			continue
		}
		key := msg.RunID + "\x00" + msg.Phase
		index, ok := assistantByRun[key]
		if !ok {
			assistantByRun[key] = len(out)
			out = append(out, msg)
			continue
		}
		existing := out[index]
		if msg.SourceKind == string(eventbus.KindAssistantCompleted) {
			existing.Text = msg.Text
		} else {
			existing.Text += msg.Text
		}
		if msg.CreatedAt.After(existing.CreatedAt) {
			existing.CreatedAt = msg.CreatedAt
		}
		out[index] = existing
	}
	return out
}
