package codexauth

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

const authFileName = "codex.json"

type Tokens struct {
	Type      string `json:"type"`
	Refresh   string `json:"refresh"`
	Access    string `json:"access"`
	Expires   int64  `json:"expires"`
	AccountID string `json:"account_id,omitempty"`
}

type Store struct {
	DataDir string
}

func (s Store) Load() (Tokens, error) {
	data, err := os.ReadFile(s.path())
	if errors.Is(err, os.ErrNotExist) {
		return Tokens{}, nil
	}
	if err != nil {
		return Tokens{}, fmt.Errorf("read codex auth: %w", err)
	}
	if strings.TrimSpace(string(data)) == "" {
		return Tokens{}, nil
	}
	var tokens Tokens
	if err := json.Unmarshal(data, &tokens); err != nil {
		return Tokens{}, fmt.Errorf("parse codex auth: %w", err)
	}
	return normalize(tokens), nil
}

func (s Store) Save(tokens Tokens) error {
	tokens = normalize(tokens)
	if err := os.MkdirAll(filepath.Dir(s.path()), 0o700); err != nil {
		return fmt.Errorf("codex auth dir: %w", err)
	}
	data, err := json.MarshalIndent(tokens, "", "  ")
	if err != nil {
		return fmt.Errorf("encode codex auth: %w", err)
	}
	data = append(data, '\n')
	if err := os.WriteFile(s.path(), data, 0o600); err != nil {
		return fmt.Errorf("write codex auth: %w", err)
	}
	return nil
}

func (s Store) Delete() error {
	if err := os.Remove(s.path()); errors.Is(err, os.ErrNotExist) {
		return nil
	} else if err != nil {
		return fmt.Errorf("delete codex auth: %w", err)
	}
	return nil
}

func (s Store) SignedIn() bool {
	tokens, err := s.Load()
	return err == nil && tokens.Refresh != ""
}

func (s Store) path() string {
	return filepath.Join(s.DataDir, "auth", authFileName)
}

func normalize(tokens Tokens) Tokens {
	if tokens.Type == "" {
		tokens.Type = "oauth"
	}
	tokens.Refresh = strings.TrimSpace(tokens.Refresh)
	tokens.Access = strings.TrimSpace(tokens.Access)
	tokens.AccountID = strings.TrimSpace(tokens.AccountID)
	return tokens
}
