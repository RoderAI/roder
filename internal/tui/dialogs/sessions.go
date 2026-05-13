package dialogs

type SessionItem struct {
	ID       string
	Title    string
	Provider string
	Model    string
	Current  bool
	Selected bool
}

type Sessions struct {
	Open     bool
	Items    []SessionItem
	Selected int
	Err      string
}

func NewSessions(items []SessionItem) Sessions {
	return Sessions{Open: true, Items: append([]SessionItem(nil), items...)}
}

func (s *Sessions) Move(delta int) {
	if len(s.Items) == 0 {
		return
	}
	s.Selected = wrapIndex(s.Selected+delta, len(s.Items))
}

func (s Sessions) SelectedItem() SessionItem {
	if len(s.Items) == 0 {
		return SessionItem{}
	}
	return s.Items[clamp(s.Selected, 0, len(s.Items)-1)]
}
