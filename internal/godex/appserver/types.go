package appserver

type Options struct {
	Version string
	Remote  RemoteOptions
}

type RemoteOptions struct {
	Enabled        bool
	Auth           RemoteAuth
	AllowedOrigins []string
	ServerName     string
}

type RemoteListenConfig struct {
	Enabled        bool
	AuthToken      string
	PrintQR        bool
	AllowedOrigins []string
}

type ClientInfo struct {
	Name    string `json:"name"`
	Title   string `json:"title,omitempty"`
	Version string `json:"version,omitempty"`
}

type InitializeParams struct {
	ClientInfo   ClientInfo             `json:"clientInfo"`
	Capabilities InitializeCapabilities `json:"capabilities,omitempty"`
}

type InitializeCapabilities struct {
	ExperimentalAPI           bool     `json:"experimentalApi,omitempty"`
	OptOutNotificationMethods []string `json:"optOutNotificationMethods,omitempty"`
}

type ThreadStatus struct {
	Type        string   `json:"type"`
	ActiveFlags []string `json:"activeFlags,omitempty"`
}

type Thread struct {
	ID            string       `json:"id"`
	SessionID     string       `json:"sessionId"`
	ForkedFromID  *string      `json:"forkedFromId"`
	Preview       string       `json:"preview"`
	Ephemeral     bool         `json:"ephemeral"`
	ModelProvider string       `json:"modelProvider"`
	CreatedAt     int64        `json:"createdAt"`
	UpdatedAt     int64        `json:"updatedAt"`
	Status        ThreadStatus `json:"status"`
	Path          *string      `json:"path"`
	CWD           string       `json:"cwd"`
	CLIVersion    string       `json:"cliVersion"`
	Source        string       `json:"source"`
	ThreadSource  *string      `json:"threadSource"`
	AgentNickname *string      `json:"agentNickname"`
	AgentRole     *string      `json:"agentRole"`
	GitInfo       any          `json:"gitInfo"`
	Name          *string      `json:"name"`
	Turns         []Turn       `json:"turns"`
}

type Turn struct {
	ID          string     `json:"id"`
	Items       []any      `json:"items"`
	ItemsView   string     `json:"itemsView"`
	Status      string     `json:"status"`
	Error       *TurnError `json:"error"`
	StartedAt   *int64     `json:"startedAt"`
	CompletedAt *int64     `json:"completedAt"`
	DurationMs  *int64     `json:"durationMs"`
}

type TurnError struct {
	Message           string `json:"message"`
	CodexErrorInfo    any    `json:"codexErrorInfo"`
	AdditionalDetails any    `json:"additionalDetails,omitempty"`
}

func idleStatus() ThreadStatus {
	return ThreadStatus{Type: "idle"}
}

func activeStatus(flags ...string) ThreadStatus {
	return ThreadStatus{Type: "active", ActiveFlags: flags}
}
