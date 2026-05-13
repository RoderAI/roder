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
	out := Commands{Open: true, Items: append([]CommandItem(nil), items...)}
	out.markSelected()
	return out
}

func (c *Commands) Move(delta int) {
	if len(c.Items) == 0 {
		return
	}
	c.Selected = wrapIndex(c.Selected+delta, len(c.Items))
	c.markSelected()
}

func (c Commands) SelectedItem() CommandItem {
	if len(c.Items) == 0 {
		return CommandItem{}
	}
	return c.Items[clamp(c.Selected, 0, len(c.Items)-1)]
}

func (c *Commands) markSelected() {
	for i := range c.Items {
		c.Items[i].Selected = i == c.Selected
	}
}
