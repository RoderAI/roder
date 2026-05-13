package tui

import (
	"strings"
	"unicode"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/tui/attachments"
	"github.com/pandelisz/gode/internal/tui/completions"
	"github.com/pandelisz/gode/internal/tui/dialogs"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

const (
	completionModeFile     = "file"
	completionModeSkill    = "skill"
	completionModeResource = "resource"
)

func (m *Model) openFileCompletions(query string) {
	m.completionMode = completionModeFile
	m.completions = dialogs.NewCommands(m.fileCompletionItems(query))
	m.completions.Query = query
	m.input.Blur()
	m.status = "file completions"
}

func (m *Model) openSkillCompletions(query string) {
	m.completionMode = completionModeSkill
	m.completions = dialogs.NewCommands(m.skillCompletionItems(query))
	m.completions.Query = query
	m.input.Blur()
	m.status = "skill completions"
}

func (m *Model) openResourceCompletions(query string) {
	m.completionMode = completionModeResource
	m.completions = dialogs.NewCommands(m.resourceCompletionItems(query))
	m.completions.Query = query
	m.input.Blur()
	m.status = "resource completions"
}

func (m *Model) openCompletionForCurrentToken() bool {
	token := m.currentCompletionToken()
	switch {
	case strings.HasPrefix(token, "$"):
		m.openSkillCompletions(token)
		return true
	case strings.HasPrefix(token, "@"):
		if isResourceMention(token) {
			m.openResourceCompletions(token)
			return true
		}
		m.openFileCompletions(token)
		return true
	default:
		return false
	}
}

func (m Model) updateCompletions(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	switch msg.String() {
	case "esc", "escape", "ctrl+[":
		m.completions = dialogs.Commands{}
		m.completionMode = ""
		m.status = "ready"
		return m, m.input.Focus()
	case "down", "j":
		m.completions.Move(1)
		return m, nil
	case "up", "k":
		m.completions.Move(-1)
		return m, nil
	case "enter":
		return m.acceptCompletionSelection()
	case "tab":
		return m.acceptCompletionSelection()
	case "backspace", "delete":
		var cmd tea.Cmd
		m.input.Focus()
		m.input, cmd = m.input.Update(msg)
		m.input.Blur()
		m.refreshCompletionItems()
		return m, cmd
	}
	if msg.Text != "" && !unicode.IsControl([]rune(msg.Text)[0]) {
		m.input.InsertString(msg.Text)
		m.refreshCompletionItems()
		return m, nil
	}
	return m, nil
}

func (m Model) updateCompletionsMouse(msg tea.MouseClickMsg) (tea.Model, tea.Cmd) {
	for i, item := range m.completions.Items {
		z := m.zones.Get(viewmodel.DialogItemZoneID("completions", item.ID))
		if z != nil && z.InBounds(msg) {
			m.completions.Selected = i
			return m.acceptCompletionSelection()
		}
	}
	return m, nil
}

func (m Model) acceptCompletionSelection() (tea.Model, tea.Cmd) {
	item := m.completions.SelectedItem()
	if item.ID == "" {
		m.completions.Err = "no completion selected"
		return m, nil
	}
	switch m.completionMode {
	case completionModeSkill:
		m.replaceCompletionToken("$" + item.ID + " ")
		m.status = "skill inserted"
	case completionModeFile:
		m.replaceCompletionToken("@" + item.ID + " ")
		m.attachments = append(m.attachments, attachments.New(item.ID))
		m.status = "file attached"
	case completionModeResource:
		m.replaceCompletionToken("@" + item.ID + " ")
		m.status = "resource inserted"
	default:
		m.replaceCompletionToken(item.ID)
		m.status = "completion inserted"
	}
	m.completions = dialogs.Commands{}
	m.completionMode = ""
	return m, m.input.Focus()
}

func (m Model) completionsViewModel() *viewmodel.ListDialog {
	if !m.completions.Open {
		return nil
	}
	items := make([]viewmodel.ListDialogItem, 0, len(m.completions.Items))
	for _, item := range m.completions.Items {
		items = append(items, viewmodel.ListDialogItem{
			ID:          item.ID,
			Label:       item.Title,
			Description: item.Description,
			Value:       item.Source,
			Selected:    item.Selected,
		})
	}
	return &viewmodel.ListDialog{
		Kind:  "completions",
		Title: completionTitle(m.completionMode),
		Help:  "enter insert  esc close  up/down navigate",
		Items: items,
		Error: m.completions.Err,
	}
}

func (m Model) fileCompletionItems(query string) []dialogs.CommandItem {
	if m.app == nil {
		return nil
	}
	files := completions.Files(m.app.Config.Workspace, query, 50)
	items := make([]dialogs.CommandItem, 0, len(files))
	for _, file := range files {
		items = append(items, dialogs.CommandItem{
			ID:          file.Path,
			Title:       "@" + file.Path,
			Description: "Attach workspace file",
			Source:      string(attachments.New(file.Path).Kind),
		})
	}
	return items
}

func (m Model) skillCompletionItems(query string) []dialogs.CommandItem {
	if m.app == nil {
		return nil
	}
	skills := completions.Skills(m.app.Skills(), query, 50)
	items := make([]dialogs.CommandItem, 0, len(skills))
	for _, skill := range skills {
		items = append(items, dialogs.CommandItem{
			ID:          skill.Name,
			Title:       "$" + skill.Name,
			Description: skill.Description,
			Source:      "skill",
		})
	}
	return items
}

func (m Model) resourceCompletionItems(query string) []dialogs.CommandItem {
	if m.app == nil || m.app.MCP == nil {
		return nil
	}
	resources := completions.Resources(m.app.MCP.Resources(), query, 50)
	items := make([]dialogs.CommandItem, 0, len(resources))
	for _, resource := range resources {
		id := resource.Server + ":" + resource.URI
		items = append(items, dialogs.CommandItem{
			ID:          id,
			Title:       "@" + id,
			Description: resource.Description,
			Source:      "mcp",
		})
	}
	return items
}

func (m Model) promptWithAttachments(prompt string) (string, error) {
	input, err := m.promptInputWithAttachments(prompt)
	if err != nil {
		return "", err
	}
	return input.Prompt, nil
}

func (m Model) promptInputWithAttachments(prompt string) (attachments.PromptInput, error) {
	workspace := "."
	if m.app != nil {
		workspace = m.app.Config.Workspace
	}
	out, err := attachments.BuildPromptInput(workspace, prompt, m.attachments)
	if err != nil {
		return attachments.PromptInput{}, err
	}
	return out, nil
}

func (m Model) attachmentViewModels() []viewmodel.Attachment {
	items := make([]viewmodel.Attachment, 0, len(m.attachments))
	for _, attachment := range m.attachments {
		items = append(items, viewmodel.Attachment{
			Path: attachment.Path,
			Kind: string(attachment.Kind),
		})
	}
	return items
}

func (m *Model) refreshCompletionItems() {
	query := m.currentCompletionToken()
	switch m.completionMode {
	case completionModeSkill:
		m.openSkillCompletions(query)
	case completionModeResource:
		m.openResourceCompletions(query)
	case completionModeFile:
		m.openFileCompletions(query)
	}
}

func (m Model) currentCompletionToken() string {
	return completionTokenAtCursor(m.input.Value(), m.input.Line(), m.input.Column())
}

func (m *Model) replaceCompletionToken(replacement string) {
	value, ok := replaceCompletionTokenAtCursor(m.input.Value(), m.input.Line(), m.input.Column(), replacement)
	if !ok {
		value = strings.TrimRight(m.input.Value(), " ") + replacement
	}
	m.input.SetValue(value)
}

func completionTokenAtCursor(value string, lineIndex int, col int) string {
	lines := strings.Split(value, "\n")
	if lineIndex < 0 || lineIndex >= len(lines) {
		return ""
	}
	line := []rune(lines[lineIndex])
	col = clamp(col, 0, len(line))
	start, end, ok := completionTokenBounds(line, col)
	if !ok {
		return ""
	}
	return string(line[start:end])
}

func replaceCompletionTokenAtCursor(value string, lineIndex int, col int, replacement string) (string, bool) {
	lines := strings.Split(value, "\n")
	if lineIndex < 0 || lineIndex >= len(lines) {
		return value, false
	}
	line := []rune(lines[lineIndex])
	col = clamp(col, 0, len(line))
	start, end, ok := completionTokenBounds(line, col)
	if !ok {
		return value, false
	}
	nextLine := string(line[:start]) + replacement + string(line[end:])
	lines[lineIndex] = nextLine
	return strings.Join(lines, "\n"), true
}

func completionTokenBounds(line []rune, col int) (int, int, bool) {
	if len(line) == 0 {
		return 0, 0, false
	}
	start := col
	for start > 0 && !unicode.IsSpace(line[start-1]) {
		start--
	}
	end := col
	for end < len(line) && !unicode.IsSpace(line[end]) {
		end++
	}
	if start == end {
		return 0, 0, false
	}
	return start, end, true
}

func completionTitle(mode string) string {
	switch mode {
	case completionModeSkill:
		return "Skills"
	case completionModeResource:
		return "Resources"
	case completionModeFile:
		return "Files"
	default:
		return "Completions"
	}
}

func isResourceMention(text string) bool {
	text = strings.TrimPrefix(strings.TrimSpace(text), "@")
	return strings.Contains(text, ":")
}
