package dialogs

import (
	"fmt"

	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/codexauth"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

type Screen int

const (
	ScreenMenu Screen = iota
	ScreenModels
	ScreenReasoning
	ScreenConfig
	ScreenSkills
	ScreenSkillRecommendations
	ScreenSkillInstall
)

type Settings struct {
	Open                bool
	Screen              Screen
	Config              godex.Config
	Models              []godex.ModelConfig
	MenuIndex           int
	ModelIndex          int
	ReasoningIndex      int
	SkillIndex          int
	RecommendedIndex    int
	Skills              []viewmodel.SettingsSkillItem
	RecommendedSkills   []viewmodel.SettingsRecommendedSkillItem
	InstallPrompt       viewmodel.SettingsInstallPrompt
	SkillInstalledCount int
	SkillEnabledCount   int
	Err                 string
}

func NewSettings(cfg godex.Config) Settings {
	settings := Settings{
		Open:   true,
		Screen: ScreenMenu,
		Config: cfg,
		Models: godex.BuiltInModels(false),
	}
	settings.selectCurrentModel()
	return settings
}

func (s *Settings) OpenMenu() {
	s.Screen = ScreenMenu
	s.Err = ""
}

func (s *Settings) OpenModels() {
	s.Screen = ScreenModels
	s.Err = ""
	s.selectCurrentModel()
}

func (s *Settings) BackToModels() {
	s.Screen = ScreenModels
	s.Err = ""
}

func (s *Settings) OpenReasoning() {
	s.Screen = ScreenReasoning
	s.Err = ""
	s.selectCurrentReasoning()
}

func (s *Settings) OpenConfig() {
	s.Screen = ScreenConfig
	s.Err = ""
}

func (s *Settings) OpenSkills() {
	s.Screen = ScreenSkills
	s.Err = ""
	if s.SkillIndex >= len(s.Skills) {
		s.SkillIndex = 0
	}
}

func (s *Settings) OpenSkillRecommendations() {
	s.Screen = ScreenSkillRecommendations
	s.Err = ""
	if s.RecommendedIndex >= len(s.RecommendedSkills) {
		s.RecommendedIndex = 0
	}
}

func (s *Settings) OpenSkillInstall() {
	s.Screen = ScreenSkillInstall
	s.Err = ""
	s.InstallPrompt.Open = true
}

func (s *Settings) Move(delta int) {
	count := s.SelectionCount()
	if count == 0 {
		return
	}
	switch s.Screen {
	case ScreenMenu:
		s.MenuIndex = wrapIndex(s.MenuIndex+delta, count)
	case ScreenModels:
		s.ModelIndex = wrapIndex(s.ModelIndex+delta, count)
	case ScreenReasoning:
		s.ReasoningIndex = wrapIndex(s.ReasoningIndex+delta, count)
	case ScreenSkills:
		s.SkillIndex = wrapIndex(s.SkillIndex+delta, count)
	case ScreenSkillRecommendations:
		s.RecommendedIndex = wrapIndex(s.RecommendedIndex+delta, count)
	}
}

func (s Settings) SelectionCount() int {
	switch s.Screen {
	case ScreenMenu:
		return len(s.MenuItems())
	case ScreenModels:
		return len(s.Models)
	case ScreenReasoning:
		return len(s.ReasoningOptions())
	case ScreenSkills:
		return len(s.Skills)
	case ScreenSkillRecommendations:
		return len(s.RecommendedSkills)
	default:
		return 0
	}
}

func (s Settings) SelectedMenuID() string {
	items := s.MenuItems()
	if len(items) == 0 {
		return ""
	}
	index := clamp(s.MenuIndex, 0, len(items)-1)
	return items[index].ID
}

func (s *Settings) selectCurrentModel() {
	s.ModelIndex = 0
	for i, model := range s.Models {
		if model.ID == s.Config.Model {
			s.ModelIndex = i
			return
		}
	}
}

func (s *Settings) selectCurrentReasoning() {
	s.ReasoningIndex = 0
	options := s.ReasoningOptions()
	selectedModel := s.SelectedModel()
	preferred := s.PreferredReasoning(selectedModel)
	for i, option := range options {
		if option.Effort == preferred {
			s.ReasoningIndex = i
			return
		}
	}
}

func (s Settings) PreferredReasoning(model godex.ModelConfig) string {
	if len(model.SupportedReasoning) == 0 {
		return ""
	}
	preferred := s.Config.Reasoning
	if preferred == "" || !model.SupportsReasoning(preferred) {
		preferred = model.DefaultReasoning
	}
	if preferred == "" && len(model.SupportedReasoning) > 0 {
		preferred = model.SupportedReasoning[0].Effort
	}
	return preferred
}

func (s Settings) SelectedModel() godex.ModelConfig {
	if len(s.Models) == 0 {
		return godex.DefaultModelConfig()
	}
	index := clamp(s.ModelIndex, 0, len(s.Models)-1)
	return s.Models[index]
}

func (s Settings) ReasoningOptions() []godex.ReasoningOption {
	model := s.SelectedModel()
	if len(model.SupportedReasoning) == 0 {
		return nil
	}
	return model.SupportedReasoning
}

func (s Settings) SelectedReasoningEffort() string {
	options := s.ReasoningOptions()
	if len(options) == 0 {
		return ""
	}
	index := clamp(s.ReasoningIndex, 0, len(options)-1)
	return options[index].Effort
}

func (s Settings) SelectedSkill() viewmodel.SettingsSkillItem {
	if len(s.Skills) == 0 {
		return viewmodel.SettingsSkillItem{}
	}
	index := clamp(s.SkillIndex, 0, len(s.Skills)-1)
	return s.Skills[index]
}

func (s Settings) MenuItems() []viewmodel.SettingsMenuItem {
	codexStatus := "signed out"
	if (codexauth.Store{DataDir: s.Config.DataDir}).SignedIn() {
		codexStatus = "signed in"
	}
	return []viewmodel.SettingsMenuItem{
		{
			ID:          "models",
			Label:       "Models",
			Description: "Choose the default model for new gode sessions.",
			Value:       godex.DisplayModelLabel(s.Config) + " / " + s.Config.Reasoning,
		},
		{
			ID:          "fast-mode",
			Label:       "Fast Mode",
			Description: "Use OpenAI priority processing for model requests.",
			Value:       onOff(s.Config.FastMode),
		},
		{
			ID:          "permission-mode",
			Label:       "Permission Mode",
			Description: "Choose whether mutating tools ask first or run without prompts.",
			Value:       permissionModeLabel(s.Config.AutoApprove),
		},
		{
			ID:          "timeline-style",
			Label:       "Timeline Style",
			Description: "Choose detailed tool output or compact tool summaries.",
			Value:       timelineStyleLabel(s.Config.TimelineStyle),
		},
		{
			ID:          "markdown-rendering",
			Label:       "Markdown Rendering",
			Description: "Render assistant and system markdown with Glamour.",
			Value:       onOff(s.Config.MarkdownRendering),
		},
		{
			ID:          "config",
			Label:       "Config",
			Description: "Review provider, reasoning, workspace, and data paths.",
			Value:       godex.DisplayProvider(s.Config),
		},
		{
			ID:          "skills",
			Label:       "Skills",
			Description: "View, enable, disable, and install agent skills.",
			Value:       skillCountValue(s.SkillEnabledCount, s.SkillInstalledCount),
		},
		{
			ID:          "codex-auth",
			Label:       "Codex Sign In",
			Description: "Connect ChatGPT Codex so GPT models use Codex auth.",
			Value:       codexStatus,
		},
	}
}

func (s Settings) ViewModel() *viewmodel.SettingsDialog {
	if !s.Open {
		return nil
	}

	vm := &viewmodel.SettingsDialog{
		Title:             s.title(),
		Screen:            s.ScreenName(),
		MenuItems:         s.viewMenuItems(),
		Models:            s.viewModels(),
		Reasoning:         s.viewReasoning(),
		ConfigRows:        s.configRows(),
		Skills:            s.viewSkills(),
		RecommendedSkills: s.viewRecommendedSkills(),
		InstallPrompt:     s.InstallPrompt,
		Selected:          s.selectedIndex(),
		Error:             s.Err,
	}
	return vm
}

func (s Settings) title() string {
	switch s.Screen {
	case ScreenModels:
		return "Models"
	case ScreenReasoning:
		return "Reasoning"
	case ScreenConfig:
		return "Config"
	case ScreenSkills:
		return "Installed Skills"
	case ScreenSkillRecommendations:
		return "Recommended Skills"
	case ScreenSkillInstall:
		return "Install Skill"
	default:
		return "Settings"
	}
}

func (s Settings) ScreenName() string {
	switch s.Screen {
	case ScreenModels:
		return viewmodel.SettingsScreenModels
	case ScreenReasoning:
		return viewmodel.SettingsScreenReasoning
	case ScreenConfig:
		return viewmodel.SettingsScreenConfig
	case ScreenSkills:
		return viewmodel.SettingsScreenSkills
	case ScreenSkillRecommendations:
		return viewmodel.SettingsScreenSkillRecs
	case ScreenSkillInstall:
		return viewmodel.SettingsScreenSkillInstall
	default:
		return viewmodel.SettingsScreenMenu
	}
}

func (s Settings) selectedIndex() int {
	switch s.Screen {
	case ScreenModels:
		return s.ModelIndex
	case ScreenReasoning:
		return s.ReasoningIndex
	case ScreenSkills:
		return s.SkillIndex
	case ScreenSkillRecommendations:
		return s.RecommendedIndex
	default:
		return s.MenuIndex
	}
}

func (s Settings) viewMenuItems() []viewmodel.SettingsMenuItem {
	items := s.MenuItems()
	for i := range items {
		items[i].Selected = i == s.MenuIndex
	}
	return items
}

func (s Settings) viewModels() []viewmodel.SettingsModelItem {
	items := make([]viewmodel.SettingsModelItem, 0, len(s.Models))
	for i, model := range s.Models {
		items = append(items, viewmodel.SettingsModelItem{
			ID:               model.ID,
			DisplayName:      model.DisplayName,
			Description:      model.Description,
			Provider:         model.Provider,
			DefaultReasoning: model.DefaultReasoning,
			Current:          model.ID == s.Config.Model,
			Selected:         i == s.ModelIndex,
		})
	}
	return items
}

func (s Settings) viewReasoning() []viewmodel.SettingsReasoningItem {
	options := s.ReasoningOptions()
	items := make([]viewmodel.SettingsReasoningItem, 0, len(options))
	selectedModel := s.SelectedModel()
	current := ""
	if s.Config.Model == selectedModel.ID {
		current = s.Config.Reasoning
	}
	for i, option := range options {
		items = append(items, viewmodel.SettingsReasoningItem{
			Effort:      option.Effort,
			Label:       reasoningLabel(option.Effort),
			Description: option.Description,
			Current:     option.Effort == current,
			Selected:    i == s.ReasoningIndex,
		})
	}
	return items
}

func (s Settings) viewSkills() []viewmodel.SettingsSkillItem {
	items := append([]viewmodel.SettingsSkillItem(nil), s.Skills...)
	for i := range items {
		items[i].Selected = i == s.SkillIndex
	}
	return items
}

func (s Settings) viewRecommendedSkills() []viewmodel.SettingsRecommendedSkillItem {
	items := append([]viewmodel.SettingsRecommendedSkillItem(nil), s.RecommendedSkills...)
	for i := range items {
		items[i].Selected = i == s.RecommendedIndex
	}
	return items
}

func (s Settings) configRows() []viewmodel.SettingsConfigRow {
	return []viewmodel.SettingsConfigRow{
		{Label: "Model", Value: s.Config.Model},
		{Label: "Provider", Value: godex.DisplayProvider(s.Config)},
		{Label: "Reasoning", Value: s.Config.Reasoning},
		{Label: "Fast mode", Value: onOff(s.Config.FastMode)},
		{Label: "Permission mode", Value: permissionModeLabel(s.Config.AutoApprove)},
		{Label: "Timeline style", Value: timelineStyleLabel(s.Config.TimelineStyle)},
		{Label: "Markdown rendering", Value: onOff(s.Config.MarkdownRendering)},
		{Label: "Workspace", Value: s.Config.Workspace},
		{Label: "Data dir", Value: s.Config.DataDir},
	}
}

func skillCountValue(enabled int, installed int) string {
	return fmt.Sprintf("%d/%d enabled", enabled, installed)
}

func settingsFromConfig(cfg godex.Config) godex.Settings {
	return godex.Settings{
		DefaultModel:          cfg.Model,
		DefaultReasoning:      cfg.Reasoning,
		FastMode:              cfg.FastMode,
		AutoApprove:           cfg.AutoApprove,
		TimelineStyle:         cfg.TimelineStyle,
		MarkdownRendering:     cfg.MarkdownRendering,
		DisableAutoCompaction: cfg.DisableAutoCompaction,
		AutoCompactTokenLimit: cfg.AutoCompactTokenLimit,
	}
}

func onOff(enabled bool) string {
	if enabled {
		return "on"
	}
	return "off"
}

func permissionModeLabel(autoApprove bool) string {
	if autoApprove {
		return "allow all"
	}
	return "request"
}

func timelineStyleLabel(style string) string {
	switch godex.NormalizeTimelineStyle(style) {
	case godex.TimelineStyleMinimal:
		return "minimal"
	default:
		return "detailed"
	}
}

func reasoningLabel(effort string) string {
	switch effort {
	case godex.ReasoningNone:
		return "None"
	case godex.ReasoningMinimal:
		return "Minimal"
	case godex.ReasoningLow:
		return "Low"
	case godex.ReasoningMedium:
		return "Medium"
	case godex.ReasoningHigh:
		return "High"
	case godex.ReasoningXHigh:
		return "XHigh"
	default:
		return effort
	}
}
