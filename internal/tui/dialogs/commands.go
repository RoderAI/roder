package dialogs

type CommandItem struct {
	ID          string
	Title       string
	Description string
	Source      string
	Selected    bool
}

type Commands struct {
	Open     bool
	Items    []CommandItem
	Selected int
	Query    string
	Err      string
}

func NewCommands(items []CommandItem) Commands {
	return Commands{Open: true, Items: append([]CommandItem(nil), items...)}
}

func (c *Commands) Move(delta int) {
	if len(c.Items) == 0 {
		return
	}
	c.Selected = wrapIndex(c.Selected+delta, len(c.Items))
}

func (c Commands) SelectedItem() CommandItem {
	if len(c.Items) == 0 {
		return CommandItem{}
	}
	return c.Items[clamp(c.Selected, 0, len(c.Items)-1)]
}
