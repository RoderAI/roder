package session

import "time"

const (
	TurnStatusRunning   = "running"
	TurnStatusCompleted = "completed"
	TurnStatusFailed    = "failed"
)

type Turn struct {
	ID          string    `json:"id"`
	SessionID   string    `json:"session_id"`
	Prompt      string    `json:"prompt"`
	Model       string    `json:"model"`
	Provider    string    `json:"provider"`
	ResponseID  string    `json:"response_id,omitempty"`
	Status      string    `json:"status"`
	Error       string    `json:"error,omitempty"`
	StartedAt   time.Time `json:"started_at"`
	CompletedAt time.Time `json:"completed_at,omitempty"`
	UpdatedAt   time.Time `json:"updated_at"`
}
