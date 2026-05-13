package dialogs

import "fmt"

const NewSessionID = "__new_session__"

type SessionItem struct {
	ID           string
	Title        string
	Provider     string
	Model        string
	MessageCount int
	Current      bool
	Selected     bool
}

type Sessions struct {
	Open     bool
	Items    []SessionItem
	Selected int
	Err      string
}

func NewSessions(items []SessionItem) Sessions {
	out := Sessions{Open: true, Items: append([]SessionItem(nil), items...)}
	out.markSelected()
	return out
}

func (s *Sessions) Move(delta int) {
	if len(s.Items) == 0 {
		return
	}
	s.Selected = wrapIndex(s.Selected+delta, len(s.Items))
	s.markSelected()
}

func (s Sessions) SelectedItem() SessionItem {
	if len(s.Items) == 0 {
		return SessionItem{}
	}
	return s.Items[clamp(s.Selected, 0, len(s.Items)-1)]
}

func (s SessionItem) Value() string {
	if s.ID == NewSessionID {
		return "start"
	}
	value := fmt.Sprintf("%d msg", s.MessageCount)
	if s.Current {
		value += " current"
	}
	return value
}

func (s *Sessions) markSelected() {
	for i := range s.Items {
		s.Items[i].Selected = i == s.Selected
	}
}
