package tui

import (
	"context"
	"fmt"
	"path/filepath"
	"strings"
	"time"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/session"
)

const resumePickerMaxRows = 8

var (
	resumePickerTitleStyle = lipgloss.NewStyle().
				Bold(true).
				Foreground(lipgloss.Color("212"))
	resumePickerHelpStyle = lipgloss.NewStyle().
				Foreground(lipgloss.Color("244"))
	resumePickerMetaStyle = lipgloss.NewStyle().
				Foreground(lipgloss.Color("246"))
	resumePickerDimStyle = lipgloss.NewStyle().
				Foreground(lipgloss.Color("240"))
	resumePickerSelectedStyle = lipgloss.NewStyle().
					Bold(true).
					Foreground(lipgloss.Color("152"))
)

type resumePickerModel struct {
	sessions     []session.Session
	workspace    string
	query        string
	scopeCurrent bool
	selected     int
	chosenID     string
	err          string
}

func PickResumeSession(ctx context.Context, app *godex.App) (string, error) {
	model := newResumePickerModel(app)
	program := tea.NewProgram(model, tea.WithContext(ctx))
	finalModel, err := program.Run()
	if err != nil {
		return "", err
	}
	final, ok := finalModel.(resumePickerModel)
	if !ok {
		return "", nil
	}
	return final.chosenID, nil
}

func newResumePickerModel(app *godex.App) resumePickerModel {
	model := resumePickerModel{scopeCurrent: true}
	if app == nil {
		model.err = "app unavailable"
		return model
	}
	model.workspace = normalizeWorkspace(app.Config.Workspace)
	if app.Sessions == nil {
		model.err = "session store unavailable"
		return model
	}
	sessions, err := app.Sessions.List(context.Background())
	if err != nil {
		model.err = err.Error()
		return model
	}
	model.sessions = sessions
	if len(model.filtered()) == 0 && len(sessions) > 0 {
		model.scopeCurrent = false
	}
	return model
}

func (m resumePickerModel) Init() tea.Cmd {
	return nil
}

func (m resumePickerModel) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.KeyPressMsg:
		switch msg.String() {
		case "ctrl+c", "esc", "escape":
			return m, tea.Quit
		case "enter":
			items := m.filtered()
			if len(items) == 0 {
				return m, nil
			}
			m.selected = clamp(m.selected, 0, len(items)-1)
			m.chosenID = items[m.selected].ID
			return m, tea.Quit
		case "down", "ctrl+n":
			m.move(1)
			return m, nil
		case "up", "ctrl+p":
			m.move(-1)
			return m, nil
		case "tab":
			m.scopeCurrent = !m.scopeCurrent
			m.selected = 0
			return m, nil
		case "backspace", "ctrl+h":
			if m.query != "" {
				runes := []rune(m.query)
				m.query = string(runes[:len(runes)-1])
				m.selected = 0
			}
			return m, nil
		case "ctrl+u":
			m.query = ""
			m.selected = 0
			return m, nil
		}
		if text := msg.String(); isSearchInput(text) {
			m.query += text
			m.selected = 0
			return m, nil
		}
	}
	return m, nil
}

func (m resumePickerModel) View() tea.View {
	return tea.NewView(m.viewString())
}

func (m resumePickerModel) viewString() string {
	width := 92
	items := m.filtered()
	if m.selected >= len(items) {
		m.selected = max(0, len(items)-1)
	}

	var lines []string
	lines = append(lines, resumePickerTitleStyle.Render("gode resume")+"  "+resumePickerScope(m.scopeCurrent))
	lines = append(lines, "search: "+resumePickerSearchValue(m.query))
	if m.err != "" {
		lines = append(lines, resumePickerDimStyle.Render(m.err))
	}
	if len(items) == 0 {
		lines = append(lines, resumePickerDimStyle.Render("No sessions match. Press tab to switch current/all or type to search."))
	} else {
		limit := min(resumePickerMaxRows, len(items))
		for i := 0; i < limit; i++ {
			lines = append(lines, renderResumePickerRow(items[i], i == m.selected, width))
		}
		if len(items) > limit {
			lines = append(lines, resumePickerDimStyle.Render(fmt.Sprintf("... %d more", len(items)-limit)))
		}
	}
	lines = append(lines, resumePickerHelpStyle.Render("enter resume  tab current/all  up/down move  ctrl+u clear  esc cancel"))
	return strings.Join(lines, "\n")
}

func (m resumePickerModel) move(delta int) {
	items := m.filtered()
	if len(items) == 0 {
		m.selected = 0
		return
	}
	m.selected = wrapResumePickerIndex(m.selected+delta, len(items))
}

func (m resumePickerModel) filtered() []session.Session {
	query := strings.ToLower(strings.TrimSpace(m.query))
	out := make([]session.Session, 0, len(m.sessions))
	for _, item := range m.sessions {
		if m.scopeCurrent && normalizeWorkspace(item.Workspace) != m.workspace {
			continue
		}
		if query != "" && !resumeSessionMatches(item, query) {
			continue
		}
		out = append(out, item)
	}
	return out
}

func resumeSessionMatches(item session.Session, query string) bool {
	haystack := strings.ToLower(strings.Join([]string{
		item.ID,
		item.Title,
		item.Workspace,
		item.Provider,
		item.Model,
	}, " "))
	return strings.Contains(haystack, query)
}

func renderResumePickerRow(item session.Session, selected bool, width int) string {
	prefix := "  "
	style := lipgloss.NewStyle()
	if selected {
		prefix = "> "
		style = resumePickerSelectedStyle
	}
	title := strings.TrimSpace(item.Title)
	if title == "" {
		title = item.ID
	}
	meta := strings.TrimSpace(strings.Join([]string{
		shortTime(item.UpdatedAt),
		fmt.Sprintf("%d msg", item.MessageCount),
		workspaceLabel(item.Workspace),
	}, "  "))
	rowWidth := max(40, width-2)
	titleWidth := max(12, rowWidth-lipgloss.Width(meta)-4)
	line := prefix + padResumePickerCell(truncateResumePickerCell(title, titleWidth), titleWidth) + "  " + resumePickerMetaStyle.Render(meta)
	return style.Render(line)
}

func resumePickerScope(current bool) string {
	if current {
		return resumePickerMetaStyle.Render("[current dir]") + resumePickerDimStyle.Render(" all")
	}
	return resumePickerDimStyle.Render("current dir ") + resumePickerMetaStyle.Render("[all]")
}

func resumePickerSearchValue(query string) string {
	if query == "" {
		return resumePickerDimStyle.Render("type to filter")
	}
	return query
}

func shortTime(t time.Time) string {
	if t.IsZero() {
		return "unknown"
	}
	return t.Local().Format("Jan 02 15:04")
}

func workspaceLabel(path string) string {
	path = strings.TrimSpace(path)
	if path == "" {
		return "no workspace"
	}
	return filepath.Base(path)
}

func normalizeWorkspace(path string) string {
	path = strings.TrimSpace(path)
	if path == "" {
		return ""
	}
	abs, err := filepath.Abs(path)
	if err == nil {
		path = abs
	}
	clean, err := filepath.EvalSymlinks(path)
	if err == nil {
		path = clean
	}
	return filepath.Clean(path)
}

func isSearchInput(text string) bool {
	if len([]rune(text)) != 1 {
		return false
	}
	switch text {
	case "\t", "\n", "\r":
		return false
	default:
		return true
	}
}

func wrapResumePickerIndex(index int, count int) int {
	if count <= 0 {
		return 0
	}
	index %= count
	if index < 0 {
		index += count
	}
	return index
}

func padResumePickerCell(text string, width int) string {
	return text + strings.Repeat(" ", max(0, width-lipgloss.Width(text)))
}

func truncateResumePickerCell(text string, width int) string {
	if width <= 0 || lipgloss.Width(text) <= width {
		return text
	}
	runes := []rune(text)
	out := ""
	for _, r := range runes {
		next := out + string(r)
		if lipgloss.Width(next+"…") > width {
			break
		}
		out = next
	}
	if out == "" {
		return "…"
	}
	return out + "…"
}
