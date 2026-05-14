package dialogs

import (
	"fmt"

	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func (s *Settings) OpenMemories() {
	s.Screen = ScreenMemories
	s.Err = ""
	if s.MemoryIndex >= len(s.memoryRows()) {
		s.MemoryIndex = 0
	}
}

func (s Settings) SelectedMemoryID() string {
	rows := s.MemoryRows()
	if len(rows) == 0 {
		return ""
	}
	index := clamp(s.MemoryIndex, 0, len(rows)-1)
	return rows[index].ID
}

func (s Settings) MemoryRows() []viewmodel.SettingsMemoryRow {
	return s.memoryRows()
}

func (s Settings) viewMemory() viewmodel.SettingsMemoryState {
	rows := s.memoryRows()
	for i := range rows {
		rows[i].Selected = i == s.MemoryIndex
	}
	return viewmodel.SettingsMemoryState{Rows: rows}
}

func (s Settings) memoryRows() []viewmodel.SettingsMemoryRow {
	return []viewmodel.SettingsMemoryRow{
		{
			ID:          "enabled",
			Label:       "Enabled",
			Value:       onOff(s.Config.Memories.Enabled),
			Description: "Toggle memory tools, prompt recall, and observer behavior.",
		},
		{
			ID:          "auto-recall",
			Label:       "Auto recall",
			Value:       onOff(s.Config.Memories.AutoRecall),
			Description: "Inject relevant workspace memories before provider calls.",
		},
		{
			ID:          "auto-observe",
			Label:       "Auto observe",
			Value:       onOff(s.Config.Memories.AutoObserve),
			Description: "Allow background observation to save durable facts after tool-heavy runs.",
		},
		{
			ID:          "count",
			Label:       "Workspace memories",
			Value:       fmt.Sprintf("%d", s.MemoryCount),
			Description: "Active memories stored for this workspace.",
		},
		{
			ID:          "database",
			Label:       "Database",
			Value:       s.Config.Memories.DatabasePath,
			Description: "Local SQLite database path.",
		},
		{
			ID:          "embedding-model",
			Label:       "Embedding model",
			Value:       s.Config.Memories.EmbeddingModel,
			Description: "Embedding model used for semantic search.",
		},
		{
			ID:          "recall-limit",
			Label:       "Recall limit",
			Value:       fmt.Sprintf("%d", s.Config.Memories.RecallLimit),
			Description: "Maximum memories injected into each prompt.",
		},
	}
}
