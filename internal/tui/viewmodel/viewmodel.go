package viewmodel

const TranscriptZoneID = "transcript"

type Role string

const (
	RoleUser      Role = "user"
	RoleAssistant Role = "assistant"
	RoleTool      Role = "tool"
	RoleSystem    Role = "system"
	RoleError     Role = "error"
)

type Message struct {
	ID    string
	Role  Role
	Title string
	Body  string
}

func MessageZoneID(id string) string {
	return "message:" + id
}

type Model struct {
	Width            int
	Height           int
	Provider         string
	Model            string
	Reasoning        string
	Messages         []Message
	ReasoningSummary string
	Input            string
	InputHeight      int
	ScrollOffset     int
	FollowTail       bool
	Running          bool
	HoveredID        string
	Status           string
	Dialogs          DialogStack
	Settings         *SettingsDialog
	ErrorLog         []ErrorLogEntry
	ShowErrorLog     bool
}

type DialogStack struct {
	Settings *SettingsDialog
}

func (m Model) ActiveSettingsDialog() *SettingsDialog {
	if m.Dialogs.Settings != nil {
		return m.Dialogs.Settings
	}
	return m.Settings
}

type ErrorLogEntry struct {
	ID      string
	Time    string
	Source  string
	Message string
}

type SettingsDialog struct {
	Title      string
	Screen     string
	MenuItems  []SettingsMenuItem
	Models     []SettingsModelItem
	Reasoning  []SettingsReasoningItem
	ConfigRows []SettingsConfigRow
	Selected   int
	Error      string
}

type SettingsMenuItem struct {
	ID          string
	Label       string
	Description string
	Value       string
	Selected    bool
}

type SettingsModelItem struct {
	ID               string
	DisplayName      string
	Description      string
	Provider         string
	DefaultReasoning string
	Current          bool
	Selected         bool
}

type SettingsConfigRow struct {
	Label string
	Value string
}

type SettingsReasoningItem struct {
	Effort      string
	Label       string
	Description string
	Current     bool
	Selected    bool
}

const (
	SettingsScreenMenu      = "menu"
	SettingsScreenModels    = "models"
	SettingsScreenReasoning = "reasoning"
	SettingsScreenConfig    = "config"
)

func SettingsMenuItemZoneID(id string) string {
	return "settings:menu:" + id
}

func SettingsModelZoneID(id string) string {
	return "settings:model:" + id
}

func SettingsReasoningZoneID(effort string) string {
	return "settings:reasoning:" + effort
}
