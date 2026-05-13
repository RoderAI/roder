package tui

import (
	"strings"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/tui/dialogs"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

const maxSlashMenuItems = 8

func (m Model) inlineSlashMenuOpen() bool {
	return len(m.slashMenuItems()) > 0
}

func (m Model) slashMenuViewModel() *viewmodel.ListDialog {
	items := m.slashMenuItems()
	if len(items) == 0 {
		return nil
	}
	viewItems := make([]viewmodel.ListDialogItem, 0, len(items))
	selected := clamp(m.slashSelected, 0, len(items)-1)
	for i, item := range items {
		viewItems = append(viewItems, viewmodel.ListDialogItem{
			ID:          item.ID,
			Label:       item.Title,
			Description: item.Description,
			Value:       item.Source,
			Selected:    i == selected,
		})
	}
	return &viewmodel.ListDialog{
		Kind:  "slash",
		Title: "Commands",
		Items: viewItems,
	}
}

func (m Model) slashMenuItems() []dialogs.CommandItem {
	query, ok := m.slashQuery()
	if !ok {
		return nil
	}
	query = strings.ToLower(query)
	for _, item := range m.commandItems() {
		if slashCommandExact(item, query) {
			return nil
		}
	}
	items := make([]dialogs.CommandItem, 0, maxSlashMenuItems)
	for _, item := range m.commandItems() {
		if slashCommandMatches(item, query) {
			items = append(items, item)
			if len(items) >= maxSlashMenuItems {
				break
			}
		}
	}
	return items
}

func (m Model) slashQuery() (string, bool) {
	value := strings.TrimLeft(m.input.Value(), " \t")
	if value == "" || !strings.HasPrefix(value, "/") {
		return "", false
	}
	if strings.Contains(value, "\n") {
		return "", false
	}
	if strings.ContainsAny(value, " \t") {
		return "", false
	}
	token := strings.TrimPrefix(value, "/")
	if strings.Contains(token, "/") {
		return "", false
	}
	return token, true
}

func slashCommandMatches(item dialogs.CommandItem, query string) bool {
	if query == "" {
		return true
	}
	title := strings.ToLower(strings.TrimPrefix(item.Title, "/"))
	id := strings.ToLower(item.ID)
	return strings.HasPrefix(title, query) || strings.HasPrefix(id, query)
}

func slashCommandExact(item dialogs.CommandItem, query string) bool {
	title := strings.ToLower(strings.TrimPrefix(item.Title, "/"))
	id := strings.ToLower(item.ID)
	return title == query || id == query
}

func (m *Model) moveSlashSelection(delta int) {
	items := m.slashMenuItems()
	if len(items) == 0 {
		m.slashSelected = 0
		return
	}
	m.slashSelected = wrapSlashIndex(m.slashSelected+delta, len(items))
}

func wrapSlashIndex(index int, count int) int {
	if count <= 0 {
		return 0
	}
	index %= count
	if index < 0 {
		index += count
	}
	return index
}

func (m *Model) clampSlashSelection() {
	items := m.slashMenuItems()
	if len(items) == 0 {
		m.slashSelected = 0
		return
	}
	m.slashSelected = clamp(m.slashSelected, 0, len(items)-1)
}

func (m Model) acceptSlashSelection() (tea.Model, tea.Cmd) {
	items := m.slashMenuItems()
	if len(items) == 0 {
		return m, nil
	}
	item := items[clamp(m.slashSelected, 0, len(items)-1)]
	m.insertCommandItem(item)
	m.slashSelected = 0
	m.status = "command inserted"
	return m, nil
}

func (m Model) updateSlashMenuMouse(msg tea.MouseClickMsg) (tea.Model, tea.Cmd) {
	items := m.slashMenuItems()
	for i, item := range items {
		z := m.zones.Get(viewmodel.DialogItemZoneID("slash", item.ID))
		if z != nil && z.InBounds(msg) {
			m.slashSelected = i
			return m.acceptSlashSelection()
		}
	}
	return m, nil
}
