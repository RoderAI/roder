package godex

import "strings"

const (
	TimelineStyleDetailed = "detailed"
	TimelineStyleMinimal  = "minimal"
)

func NormalizeTimelineStyle(style string) string {
	switch strings.ToLower(strings.TrimSpace(style)) {
	case TimelineStyleMinimal:
		return TimelineStyleMinimal
	case TimelineStyleDetailed:
		return TimelineStyleDetailed
	default:
		return TimelineStyleMinimal
	}
}
