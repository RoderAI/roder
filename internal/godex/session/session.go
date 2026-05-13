package session

import "time"

type ResumeMode string

const (
	ResumeModeLocalItems         ResumeMode = "local_items"
	ResumeModePreviousResponseID ResumeMode = "previous_response_id"
	ResumeModeHybrid             ResumeMode = "hybrid"
)

type Session struct {
	ID               string    `json:"id"`
	Title            string    `json:"title"`
	Workspace        string    `json:"workspace,omitempty"`
	Model            string    `json:"model,omitempty"`
	Provider         string    `json:"provider,omitempty"`
	LastResponseID   string    `json:"last_response_id,omitempty"`
	CurrentTurnID    string    `json:"current_turn_id,omitempty"`
	ParentSessionID  string    `json:"parent_session_id,omitempty"`
	MessageCount     int       `json:"message_count"`
	ItemCount        int       `json:"item_count,omitempty"`
	PromptTokens     int64     `json:"prompt_tokens,omitempty"`
	CompletionTokens int64     `json:"completion_tokens,omitempty"`
	Cost             float64   `json:"cost,omitempty"`
	CreatedAt        time.Time `json:"created_at"`
	UpdatedAt        time.Time `json:"updated_at"`
}
