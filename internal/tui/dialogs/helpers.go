package dialogs

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

func wrapIndex(index int, count int) int {
	if count <= 0 {
		return 0
	}
	index %= count
	if index < 0 {
		index += count
	}
	return index
}

func max(a int, b int) int {
	if a > b {
		return a
	}
	return b
}
