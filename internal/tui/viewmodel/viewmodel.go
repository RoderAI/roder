package viewmodel

import "github.com/pandelisz/gode/internal/tui/selection"

const TranscriptZoneID = "transcript"
const ComposerZoneID = "composer"

type Role string

const (
	RoleUser      Role = "user"
	RoleAssistant Role = "assistant"
	RoleTool      Role = "tool"
	RoleSystem    Role = "system"
	RoleError     Role = "error"
)

const (
	TimelineStyleDetailed = "detailed"
	TimelineStyleMinimal  = "minimal"
)

type Message struct {
	ID    string
	Role  Role
	Title string
	Body  string
}

type Attachment struct {
	Path string
	Kind string
}

func MessageZoneID(id string) string {
	return "message:" + id
}

type Model struct {
	Width                     int
	Height                    int
	Provider                  string
	Model                     string
	Reasoning                 string
	SessionTitle              string
	Messages                  []Message
	TimelineStyle             string
	ReasoningSummary          string
	Attachments               []Attachment
	Input                     string
	ComposerValue             string
	InputHeight               int
	SlashMenu                 *ListDialog
	ScrollOffset              int
	FollowTail                bool
	TranscriptSelection       selection.Range
	TranscriptSelectionHint   string
	TranscriptSelectionActive bool
	CopyNotice                string
	ComposerSelection         selection.OffsetRange
	ComposerSelectionHint     string
	ComposerSelectionActive   bool
	AutoApprove               bool
	Running                   bool
	HoveredID                 string
	Status                    string
	ContextLeft               string
	Dialogs                   DialogStack
	Settings                  *SettingsDialog
	ErrorLog                  []ErrorLogEntry
	ShowErrorLog              bool
}

type DialogStack struct {
	Settings    *SettingsDialog
	Completions *ListDialog
	Commands    *ListDialog
	Sessions    *ListDialog
	Permissions *PermissionDialog
}

func (m Model) ActiveSettingsDialog() *SettingsDialog {
	if m.Dialogs.Settings != nil {
		return m.Dialogs.Settings
	}
	return m.Settings
}

func (m Model) ActiveListDialog() *ListDialog {
	switch {
	case m.Dialogs.Completions != nil:
		return m.Dialogs.Completions
	case m.Dialogs.Commands != nil:
		return m.Dialogs.Commands
	case m.Dialogs.Sessions != nil:
		return m.Dialogs.Sessions
	default:
		return nil
	}
}

func (m Model) ActivePermissionDialog() *PermissionDialog {
	return m.Dialogs.Permissions
}

type ListDialog struct {
	Kind  string
	Title string
	Help  string
	Items []ListDialogItem
	Error string
}

type ListDialogItem struct {
	ID          string
	Label       string
	Description string
	Value       string
	Selected    bool
}

type PermissionDialog struct {
	Title    string
	Help     string
	Requests []PermissionDialogRequest
	Error    string
}

type PermissionDialogRequest struct {
	ID       string
	Tool     string
	Action   string
	Input    string
	Selected bool
}

type ErrorLogEntry struct {
	ID      string
	Time    string
	Source  string
	Message string
}

type SettingsDialog struct {
	Title             string
	Screen            string
	MenuItems         []SettingsMenuItem
	Models            []SettingsModelItem
	Reasoning         []SettingsReasoningItem
	ConfigRows        []SettingsConfigRow
	Skills            []SettingsSkillItem
	RecommendedSkills []SettingsRecommendedSkillItem
	InstallPrompt     SettingsInstallPrompt
	Selected          int
	Error             string
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

type SettingsSkillItem struct {
	Name        string
	Description string
	Path        string
	Source      string
	Scope       string
	State       string
	Diagnostic  string
	Enabled     bool
	Selected    bool
}

type SettingsRecommendedSkillItem struct {
	Name     string
	Source   string
	State    string
	Selected bool
}

type SettingsInstallPrompt struct {
	Open       bool
	Source     string
	Installing bool
	Error      string
}

const (
	SettingsScreenMenu         = "menu"
	SettingsScreenModels       = "models"
	SettingsScreenReasoning    = "reasoning"
	SettingsScreenConfig       = "config"
	SettingsScreenSkills       = "skills"
	SettingsScreenSkillRecs    = "skill-recommendations"
	SettingsScreenSkillInstall = "skill-install"
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

func SettingsSkillZoneID(name string) string {
	return "settings:skill:" + name
}

func SettingsRecommendedSkillZoneID(name string) string {
	return "settings:recommended-skill:" + name
}

func DialogItemZoneID(kind string, id string) string {
	return "dialog:" + kind + ":" + id
}
