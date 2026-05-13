package selection

import "strings"

const minCopyRunes = 3

type Point struct {
	Line   int
	Column int
}

type Range struct {
	Anchor Point
	Focus  Point
	Active bool
}

func (r Range) Normalize() (Point, Point, bool) {
	if !r.Active || r.Anchor == r.Focus {
		return Point{}, Point{}, false
	}
	if pointLess(r.Focus, r.Anchor) {
		return r.Focus, r.Anchor, true
	}
	return r.Anchor, r.Focus, true
}

func (r Range) SelectedText(lines []string) string {
	start, end, ok := r.Normalize()
	if !ok || len(lines) == 0 {
		return ""
	}
	if end.Line < 0 || start.Line >= len(lines) {
		return ""
	}
	start.Line = clampInt(start.Line, 0, len(lines)-1)
	end.Line = clampInt(end.Line, 0, len(lines)-1)

	parts := make([]string, 0, end.Line-start.Line+1)
	for lineIndex := start.Line; lineIndex <= end.Line; lineIndex++ {
		runes := []rune(lines[lineIndex])
		from := 0
		to := len(runes)
		if lineIndex == start.Line {
			from = clampInt(start.Column, 0, len(runes))
		}
		if lineIndex == end.Line {
			to = clampInt(end.Column, 0, len(runes))
		}
		if from > to {
			return ""
		}
		parts = append(parts, string(runes[from:to]))
	}
	return strings.Join(parts, "\n")
}

func (r Range) CharCount(lines []string) int {
	text := r.SelectedText(lines)
	count := 0
	for _, char := range text {
		if char != '\n' {
			count++
		}
	}
	return count
}

func (r Range) CanCopy(lines []string) bool {
	return r.CharCount(lines) >= minCopyRunes
}

func pointLess(left Point, right Point) bool {
	if left.Line != right.Line {
		return left.Line < right.Line
	}
	return left.Column < right.Column
}

func clampInt(value int, low int, high int) int {
	if high < low {
		return low
	}
	if value < low {
		return low
	}
	if value > high {
		return high
	}
	return value
}
