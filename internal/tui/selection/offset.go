package selection

type OffsetRange struct {
	Anchor int
	Focus  int
	Active bool
}

func (r OffsetRange) Normalize(text string) (int, int, bool) {
	if !r.Active || r.Anchor == r.Focus {
		return 0, 0, false
	}
	start, end := r.Anchor, r.Focus
	if end < start {
		start, end = end, start
	}
	length := len([]rune(text))
	start = clampInt(start, 0, length)
	end = clampInt(end, 0, length)
	if start == end {
		return 0, 0, false
	}
	return start, end, true
}

func (r OffsetRange) SelectedText(text string) string {
	start, end, ok := r.Normalize(text)
	if !ok {
		return ""
	}
	runes := []rune(text)
	return string(runes[start:end])
}

func (r OffsetRange) CharCount(text string) int {
	return len([]rune(r.SelectedText(text)))
}

func (r OffsetRange) CanCopy(text string) bool {
	return r.CharCount(text) >= minCopyRunes
}
