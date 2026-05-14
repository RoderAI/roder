package components

import (
	"strings"

	"charm.land/lipgloss/v2"
	"github.com/charmbracelet/x/ansi"
	zone "github.com/lrstanley/bubblezone/v2"
	"github.com/pandelisz/gode/internal/tui/diffview"
	"github.com/pandelisz/gode/internal/tui/selection"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

type renderedMessage struct {
	id           string
	messageIndex int
	lines        []renderedLine
}

type renderedLine struct {
	text string
	ref  selection.TranscriptLineRef
}

type TranscriptOptions struct {
	Selection         selection.Range
	SelectionStyle    lipgloss.Style
	TimelineStyle     string
	MarkdownRendering bool
}

type TranscriptRenderResult struct {
	View  string
	Lines []selection.TranscriptLineRef
}

type TranscriptCache struct {
	entries map[string]cachedMessage
}

type cachedMessage struct {
	width         int
	themeVersion  int
	timelineStyle string
	markdown      bool
	role          viewmodel.Role
	title         string
	body          string
	item          renderedMessage
}

func NewTranscriptCache() TranscriptCache {
	return TranscriptCache{entries: make(map[string]cachedMessage)}
}

func Transcript(width int, height int, messages []viewmodel.Message, scrollOffset int, hoveredID string, zones *zone.Manager) string {
	return TranscriptWithCache(width, height, messages, scrollOffset, hoveredID, zones, nil)
}

func TranscriptWithCache(width int, height int, messages []viewmodel.Message, scrollOffset int, hoveredID string, zones *zone.Manager, cache *TranscriptCache) string {
	return TranscriptDetailedWithCache(width, height, messages, scrollOffset, hoveredID, zones, cache, TranscriptOptions{}).View
}

func TranscriptDetailedWithCache(width int, height int, messages []viewmodel.Message, scrollOffset int, hoveredID string, zones *zone.Manager, cache *TranscriptCache, options TranscriptOptions) TranscriptRenderResult {
	panelHeight := max(0, height)
	if panelHeight == 0 {
		return TranscriptRenderResult{}
	}
	innerWidth := max(20, width-2)
	contentWidth := max(12, innerWidth-2)
	innerHeight := max(1, panelHeight-2)
	timelineStyle := normalizeTimelineStyle(options.TimelineStyle)
	visible := visibleMessagesWithOptions(messages, contentWidth, innerHeight, scrollOffset, cache, timelineStyle, options.MarkdownRendering)
	refs := visibleLineRefs(visible)

	var body string
	if len(visible) == 0 {
		body = emptyStyle.Render("No transcript yet. Ask gode to inspect, edit, or run something.")
	} else {
		parts := make([]string, 0, len(visible))
		for _, item := range visible {
			lines := renderTranscriptLines(item.lines, options)
			block := strings.Join(lines, "\n")
			if item.id == hoveredID {
				block = messageHoverStyle.Width(contentWidth).Render(block)
			}
			parts = append(parts, zones.Mark(viewmodel.MessageZoneID(item.id), block))
		}
		body = strings.Join(parts, "\n")
	}

	style := transcriptStyle
	if panelHeight < 3 {
		style = lipgloss.NewStyle()
	}
	panel := style.
		Width(innerWidth).
		Height(panelHeight).
		Render(body)
	return TranscriptRenderResult{
		View:  zones.Mark(viewmodel.TranscriptZoneID, panel),
		Lines: refs,
	}
}

func TranscriptLineRefs(width int, height int, messages []viewmodel.Message, scrollOffset int, cache *TranscriptCache, styles ...string) []selection.TranscriptLineRef {
	panelHeight := max(4, height)
	innerWidth := max(20, width-2)
	contentWidth := max(12, innerWidth-2)
	innerHeight := max(1, panelHeight-2)
	return visibleLineRefs(visibleMessages(messages, contentWidth, innerHeight, scrollOffset, cache, styles...))
}

func (c *TranscriptCache) Prune(messages []viewmodel.Message) {
	if c == nil || len(c.entries) == 0 {
		return
	}
	seen := make(map[string]struct{}, len(messages))
	for _, msg := range messages {
		seen[msg.ID] = struct{}{}
	}
	for id := range c.entries {
		if _, ok := seen[id]; !ok {
			delete(c.entries, id)
		}
	}
}

func visibleMessages(messages []viewmodel.Message, width int, height int, scrollOffset int, cache *TranscriptCache, styles ...string) []renderedMessage {
	timelineStyle := viewmodel.TimelineStyleMinimal
	if len(styles) > 0 {
		timelineStyle = normalizeTimelineStyle(styles[0])
	}
	return visibleMessagesWithOptions(messages, width, height, scrollOffset, cache, timelineStyle, false)
}

func visibleMessagesWithOptions(messages []viewmodel.Message, width int, height int, scrollOffset int, cache *TranscriptCache, timelineStyle string, markdown bool) []renderedMessage {
	if len(messages) == 0 || height <= 0 {
		return nil
	}
	timelineStyle = normalizeTimelineStyle(timelineStyle)
	lineBudget := max(height, height+max(0, scrollOffset))
	reversed := make([]renderedMessage, 0, min(len(messages), height))
	total := 0
	for i := len(messages) - 1; i >= 0 && total < lineBudget; i-- {
		item := renderMessageCached(messages[i], width, timelineStyle, markdown, cache)
		item.messageIndex = i
		item.lines = normalizedMessageLines(item)
		reversed = append(reversed, item)
		total += len(item.lines)
	}
	if len(reversed) == 0 {
		return nil
	}

	rendered := make([]renderedMessage, len(reversed))
	for i := range reversed {
		rendered[len(reversed)-1-i] = reversed[i]
	}
	scrollOffset = clamp(scrollOffset, 0, max(0, total-height))
	startLine := max(0, total-height-scrollOffset)
	endLine := min(total, startLine+height)

	visible := make([]renderedMessage, 0, len(rendered))
	cursor := 0
	visibleLine := 0
	for _, item := range rendered {
		itemStart := cursor
		itemEnd := cursor + len(item.lines)
		cursor = itemEnd

		if itemEnd <= startLine || itemStart >= endLine {
			continue
		}

		from := max(0, startLine-itemStart)
		to := min(len(item.lines), endLine-itemStart)
		lines := cloneRenderedLines(item.lines[from:to])
		for i := range lines {
			lines[i].ref.MessageIndex = item.messageIndex
		}
		visible = append(visible, renderedMessage{id: item.id, messageIndex: item.messageIndex, lines: lines})
	}
	visible = trimLeadingMessageGaps(visible)
	for itemIndex := range visible {
		for lineIndex := range visible[itemIndex].lines {
			visible[itemIndex].lines[lineIndex].ref.DisplayLine = visibleLine
			visibleLine++
		}
	}
	return visible
}

func normalizedMessageLines(item renderedMessage) []renderedLine {
	lines := trimEdgeBlankLines(item.lines)
	if item.messageIndex <= 0 {
		return lines
	}
	out := make([]renderedLine, 0, len(lines)+1)
	out = append(out, messageGapLine())
	out = append(out, lines...)
	return out
}

func messageGapLine() renderedLine {
	return renderedLine{
		text: "",
		ref: selection.TranscriptLineRef{
			LogicalLine: -1,
			Text:        "",
			Decorative:  true,
		},
	}
}

func trimLeadingMessageGaps(messages []renderedMessage) []renderedMessage {
	for len(messages) > 0 {
		for len(messages[0].lines) > 0 && isMessageGapLine(messages[0].lines[0]) {
			messages[0].lines = messages[0].lines[1:]
		}
		if len(messages[0].lines) > 0 {
			return messages
		}
		messages = messages[1:]
	}
	return messages
}

func isMessageGapLine(line renderedLine) bool {
	return line.ref.Decorative && line.ref.LogicalLine == -1 && line.text == ""
}

func trimEdgeBlankLines(lines []renderedLine) []renderedLine {
	start := 0
	for start < len(lines) && isBlankRenderedLine(lines[start]) {
		start++
	}
	if start == len(lines) {
		return lines[:1]
	}
	end := len(lines)
	for end > start && isBlankRenderedLine(lines[end-1]) {
		end--
	}
	if start == 0 && end == len(lines) {
		return lines
	}
	return lines[start:end]
}

func isBlankRenderedLine(line renderedLine) bool {
	return strings.TrimSpace(ansi.Strip(line.text)) == ""
}

func renderMessageCached(msg viewmodel.Message, width int, timelineStyle string, markdown bool, cache *TranscriptCache) renderedMessage {
	if cache == nil {
		return renderMessage(msg, width, timelineStyle, markdown)
	}
	if cache.entries == nil {
		cache.entries = make(map[string]cachedMessage)
	}
	if entry, ok := cache.entries[msg.ID]; ok &&
		entry.width == width &&
		entry.themeVersion == ThemeVersion() &&
		entry.timelineStyle == timelineStyle &&
		entry.markdown == markdown &&
		entry.role == msg.Role &&
		entry.title == msg.Title &&
		entry.body == msg.Body {
		return entry.item
	}
	item := renderMessage(msg, width, timelineStyle, markdown)
	cache.entries[msg.ID] = cachedMessage{
		width:         width,
		themeVersion:  ThemeVersion(),
		timelineStyle: timelineStyle,
		markdown:      markdown,
		role:          msg.Role,
		title:         msg.Title,
		body:          msg.Body,
		item:          item,
	}
	return item
}

func renderMessage(msg viewmodel.Message, width int, timelineStyle string, markdown bool) renderedMessage {
	item := renderedMessage{}
	switch msg.Role {
	case viewmodel.RoleTool:
		item = renderToolMessage(msg, width, timelineStyle)
	case viewmodel.RoleUser:
		item = renderUserMessage(msg, width)
	case viewmodel.RoleAssistant:
		item = renderAssistantMessage(msg, width, markdown)
	case viewmodel.RoleError:
		item = renderMetaMessage(msg, width, errorPrefixStyle.Render("!"), msg.Title, markdown)
	case viewmodel.RoleSystem:
		item = renderMetaMessage(msg, width, metaPrefixStyle.Render("·"), msg.Title, markdown)
	default:
		item = renderMetaMessage(msg, width, metaPrefixStyle.Render("·"), string(msg.Role), markdown)
	}
	for i := range item.lines {
		item.lines[i].ref.LogicalLine = i
		item.lines[i].ref.DisplayLine = i
	}
	return item
}

func renderUserMessage(msg viewmodel.Message, width int) renderedMessage {
	prefix := userRailStyle.Render("▌") + " "
	lines := userMessageLines(msg.Body, prefix, max(12, width-lipgloss.Width(prefix)))
	if msg.Title != "" {
		title := userMessageStyle.Render(strings.TrimSpace(msg.Title))
		header := renderedLine{
			text: prefix + title,
			ref:  selection.TranscriptLineRef{Text: prefix + title, Decorative: true},
		}
		lines = append([]renderedLine{header}, lines...)
	}
	return renderedMessage{id: msg.ID, lines: lines}
}

func renderAssistantMessage(msg viewmodel.Message, width int, markdown bool) renderedMessage {
	lines := assistantBodyLines(msg.Body, max(12, width), markdown)
	if shouldRenderAssistantTitle(msg.Title) {
		headerText := metaPrefixStyle.Render("· ") + metaTitleStyle.Render(strings.TrimSpace(msg.Title))
		header := renderedLine{
			text: headerText,
			ref:  selection.TranscriptLineRef{Text: headerText, Decorative: true},
		}
		lines = append([]renderedLine{header}, lines...)
	}
	return renderedMessage{id: msg.ID, lines: lines}
}

func shouldRenderAssistantTitle(title string) bool {
	return strings.TrimSpace(title) != "" && strings.TrimSpace(title) != "commentary"
}

func renderMetaMessage(msg viewmodel.Message, width int, prefix string, title string, markdown bool) renderedMessage {
	title = strings.TrimSpace(title)
	if title == "" {
		title = string(msg.Role)
	}
	header := prefix + " " + metaTitleStyle.Render(title)
	lines := []renderedLine{{
		text: header,
		ref:  selection.TranscriptLineRef{Text: header, Decorative: true},
	}}
	for _, line := range bodyLines(msg.Body, max(12, width-2), markdown) {
		line.text = "  " + line.text
		line.ref.Text = line.text
		lines = append(lines, line)
	}
	return renderedMessage{id: msg.ID, lines: lines}
}

func renderToolMessage(msg viewmodel.Message, width int, timelineStyle string) renderedMessage {
	title := strings.TrimSpace(msg.Title)
	if title == "" {
		title = "tool"
	}
	if timelineStyle == viewmodel.TimelineStyleMinimal {
		return renderMinimalToolMessage(msg, title, width)
	}
	prefix := toolTitleStyle.Render("› " + title)
	lines := []renderedLine{{
		text: prefix,
		ref:  selection.TranscriptLineRef{Text: prefix, Decorative: true},
	}}
	lines = append(lines, toolBodyLines(title, msg.Body, max(12, width-2))...)
	return renderedMessage{id: msg.ID, lines: lines}
}

func renderMinimalToolMessage(msg viewmodel.Message, title string, width int) renderedMessage {
	summary := firstToolSummaryLine(msg.Body)
	label := toolTitleStyle.Render("└ ● " + title)
	if summary != "" {
		label += toolMetaStyle.Render(" (" + truncateCell(summary, max(8, width-lipgloss.Width("└ ● "+title)-3)) + ")")
	}
	return renderedMessage{id: msg.ID, lines: []renderedLine{{
		text: label,
		ref: selection.TranscriptLineRef{
			Text:     label,
			CopyText: strings.TrimSpace(title + " " + summary),
		},
	}}}
}

func firstToolSummaryLine(body string) string {
	for _, line := range strings.Split(body, "\n") {
		if trimmed := strings.TrimSpace(line); trimmed != "" {
			return trimmed
		}
	}
	return ""
}

func toolBodyLines(tool string, body string, width int) []renderedLine {
	var bodyLines []string
	if diffview.IsDiffTool(tool) && looksLikeDiff(body) {
		bodyLines = diffview.RenderLines(body, width, 24)
	} else {
		bodyLines = wrapText(body, width)
	}
	if len(bodyLines) == 0 {
		return []renderedLine{bodyRenderedLine("  "+bodyStyle.Render(""), "")}
	}

	lines := make([]renderedLine, 0, len(bodyLines))
	for _, line := range bodyLines {
		lines = append(lines, bodyRenderedLine("  "+toolBodyLine(line), line))
	}
	return lines
}

func toolBodyLine(line string) string {
	key, value, ok := strings.Cut(line, ":")
	if !ok {
		return bodyStyle.Render(line)
	}
	key = strings.TrimSpace(key)
	if key == "" || len(key) > 24 {
		return bodyStyle.Render(line)
	}
	return toolMetaStyle.Render(strings.ToLower(key)+":") + " " + bodyStyle.Render(strings.TrimSpace(value))
}

func renderTranscriptLines(lines []renderedLine, options TranscriptOptions) []string {
	out := make([]string, 0, len(lines))
	style := options.SelectionStyle
	if style.GetBackground() == nil && style.GetForeground() == nil {
		style = ThemeSelectionStyle()
	}
	start, end, ok := options.Selection.Normalize()
	for _, line := range lines {
		text := line.text
		if ok && line.ref.DisplayLine >= start.Line && line.ref.DisplayLine <= end.Line {
			startCol := 0
			endCol := len([]rune(ansi.Strip(text)))
			if line.ref.DisplayLine == start.Line {
				startCol = start.Column
			}
			if line.ref.DisplayLine == end.Line {
				endCol = end.Column
			}
			text = selection.HighlightLine(text, startCol, endCol, style)
		}
		out = append(out, text)
	}
	return out
}

func visibleLineRefs(messages []renderedMessage) []selection.TranscriptLineRef {
	var refs []selection.TranscriptLineRef
	for _, item := range messages {
		for _, line := range item.lines {
			refs = append(refs, line.ref)
		}
	}
	return refs
}

func cloneRenderedLines(lines []renderedLine) []renderedLine {
	out := make([]renderedLine, len(lines))
	copy(out, lines)
	return out
}

func bodyRenderedLine(text string, copyText string) renderedLine {
	return renderedLine{
		text: text,
		ref: selection.TranscriptLineRef{
			Text:     text,
			CopyText: copyText,
		},
	}
}

func looksLikeDiff(text string) bool {
	for _, line := range strings.Split(text, "\n") {
		if strings.HasPrefix(line, "diff --git ") ||
			strings.HasPrefix(line, "@@") ||
			strings.HasPrefix(line, "+++") ||
			strings.HasPrefix(line, "---") {
			return true
		}
	}
	return false
}

func normalizeTimelineStyle(style string) string {
	switch strings.TrimSpace(strings.ToLower(style)) {
	case viewmodel.TimelineStyleDetailed:
		return viewmodel.TimelineStyleDetailed
	default:
		return viewmodel.TimelineStyleMinimal
	}
}

func wrappedBodyLines(text string, width int) []renderedLine {
	return styledWrappedBodyLines(text, width, bodyStyle)
}

func styledWrappedBodyLines(text string, width int, style lipgloss.Style) []renderedLine {
	lines := wrapText(text, width)
	out := make([]renderedLine, 0, len(lines))
	for i := range lines {
		out = append(out, bodyRenderedLine(style.Render(lines[i]), lines[i]))
	}
	return out
}

func assistantBodyLines(text string, width int, markdown bool) []renderedLine {
	if markdown {
		return markdownBodyLinesWithBaseColor(text, width, string(themeColor(ColorTextStrong)))
	}
	return styledWrappedBodyLines(text, width, assistantBodyStyle)
}

func bodyLines(text string, width int, markdown bool) []renderedLine {
	if markdown {
		return markdownBodyLinesWithBaseColor(text, width, string(themeColor(ColorText)))
	}
	return wrappedBodyLines(text, width)
}

func userMessageLines(text string, prefix string, width int) []renderedLine {
	wrapped := wrapText(text, width)
	lines := make([]renderedLine, 0, len(wrapped))
	for _, line := range wrapped {
		lines = append(lines, bodyRenderedLine(prefix+userMessageStyle.Render(line), line))
	}
	return lines
}

func clamp(v int, low int, high int) int {
	if high < low {
		return low
	}
	if v < low {
		return low
	}
	if v > high {
		return high
	}
	return v
}

func min(a, b int) int {
	if a < b {
		return a
	}
	return b
}
