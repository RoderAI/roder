package session

import "time"

type Session struct {
	ID               string    `json:"id"`
	Title            string    `json:"title"`
	ParentSessionID  string    `json:"parent_session_id,omitempty"`
	MessageCount     int       `json:"message_count"`
	PromptTokens     int64     `json:"prompt_tokens,omitempty"`
	CompletionTokens int64     `json:"completion_tokens,omitempty"`
	Cost             float64   `json:"cost,omitempty"`
	CreatedAt        time.Time `json:"created_at"`
	UpdatedAt        time.Time `json:"updated_at"`
}
