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
	MarkdownRendering         bool
	ReasoningSummary          string
	QueuedPrompts             []string
	Attachments               []Attachment
	Input                     string
	ComposerValue             string
	InputHeight               int
	SlashMenu                 *ListDialog
	CompletionMenu            *ListDialog
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
	Remote                    *RemoteDialog
	QuitDialog                *ConfirmDialog
	StopDialog                *ConfirmDialog
	ErrorLog                  []ErrorLogEntry
	ShowErrorLog              bool
}

type DialogStack struct {
	Settings    *SettingsDialog
	Completions *ListDialog
	Commands    *ListDialog
	Sessions    *ListDialog
	Permissions *PermissionDialog
	Remote      *RemoteDialog
}

func (m Model) ActiveSettingsDialog() *SettingsDialog {
	if m.Dialogs.Settings != nil {
		return m.Dialogs.Settings
	}
	return m.Settings
}

func (m Model) ActiveListDialog() *ListDialog {
	switch {
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

func (m Model) ActiveRemoteDialog() *RemoteDialog {
	if m.Dialogs.Remote != nil {
		return m.Dialogs.Remote
	}
	return m.Remote
}

type RemoteDialog struct {
	Title            string
	Running          bool
	URLs             []string
	TokenPreview     string
	QR               string
	AuthHeaderHint   string
	SubprotocolHint  string
	ConnectedClients int
	Warning          string
	Error            string
	Help             string
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

type ConfirmDialog struct {
	Title        string
	Message      string
	ConfirmLabel string
	CancelLabel  string
	Help         string
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
	Memory            SettingsMemoryState
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

type SettingsMemoryState struct {
	Rows []SettingsMemoryRow
}

type SettingsMemoryRow struct {
	ID          string
	Label       string
	Value       string
	Description string
	Selected    bool
}

type SettingsSkillItem struct {
	Name             string
	DisplayName      string
	Description      string
	ShortDescription string
	Path             string
	Source           string
	Scope            string
	State            string
	DependencyHints  []string
	Diagnostic       string
	AmbiguousName    bool
	Enabled          bool
	Selected         bool
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
	SettingsScreenMemories     = "memories"
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

func SettingsMemoryZoneID(id string) string {
	return "settings:memory:" + id
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
