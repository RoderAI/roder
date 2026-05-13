package dialogs

type Kind string

const (
	KindModels      Kind = "models"
	KindSessions    Kind = "sessions"
	KindCommands    Kind = "commands"
	KindPermissions Kind = "permissions"
)

type Stack struct {
	items []Kind
}

func (s *Stack) Push(kind Kind) {
	if kind == "" {
		return
	}
	s.items = append(s.items, kind)
}

func (s *Stack) Pop() (Kind, bool) {
	if len(s.items) == 0 {
		return "", false
	}
	last := s.items[len(s.items)-1]
	s.items = s.items[:len(s.items)-1]
	return last, true
}

func (s Stack) Top() (Kind, bool) {
	if len(s.items) == 0 {
		return "", false
	}
	return s.items[len(s.items)-1], true
}

func (s Stack) Len() int {
	return len(s.items)
}
