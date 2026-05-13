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
	Width        int
	Height       int
	Provider     string
	Model        string
	Messages     []Message
	Input        string
	InputHeight  int
	ScrollOffset int
	FollowTail   bool
	Running      bool
	HoveredID    string
	Status       string
}
