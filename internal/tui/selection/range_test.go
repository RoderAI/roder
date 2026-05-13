package selection

import "testing"

func TestRangeNormalize(t *testing.T) {
	tests := []struct {
		name      string
		rng       Range
		wantStart Point
		wantEnd   Point
		wantOK    bool
	}{
		{
			name:      "forward same line",
			rng:       Range{Anchor: Point{Line: 1, Column: 2}, Focus: Point{Line: 1, Column: 5}, Active: true},
			wantStart: Point{Line: 1, Column: 2},
			wantEnd:   Point{Line: 1, Column: 5},
			wantOK:    true,
		},
		{
			name:      "reverse same line",
			rng:       Range{Anchor: Point{Line: 1, Column: 5}, Focus: Point{Line: 1, Column: 2}, Active: true},
			wantStart: Point{Line: 1, Column: 2},
			wantEnd:   Point{Line: 1, Column: 5},
			wantOK:    true,
		},
		{
			name:      "forward multi line",
			rng:       Range{Anchor: Point{Line: 0, Column: 4}, Focus: Point{Line: 2, Column: 3}, Active: true},
			wantStart: Point{Line: 0, Column: 4},
			wantEnd:   Point{Line: 2, Column: 3},
			wantOK:    true,
		},
		{
			name:      "reverse multi line",
			rng:       Range{Anchor: Point{Line: 3, Column: 1}, Focus: Point{Line: 1, Column: 8}, Active: true},
			wantStart: Point{Line: 1, Column: 8},
			wantEnd:   Point{Line: 3, Column: 1},
			wantOK:    true,
		},
		{
			name:   "collapsed",
			rng:    Range{Anchor: Point{Line: 1, Column: 2}, Focus: Point{Line: 1, Column: 2}, Active: true},
			wantOK: false,
		},
		{
			name:   "inactive",
			rng:    Range{Anchor: Point{Line: 1, Column: 2}, Focus: Point{Line: 1, Column: 5}},
			wantOK: false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			start, end, ok := tt.rng.Normalize()
			if ok != tt.wantOK {
				t.Fatalf("ok = %v, want %v", ok, tt.wantOK)
			}
			if start != tt.wantStart || end != tt.wantEnd {
				t.Fatalf("range = %#v..%#v, want %#v..%#v", start, end, tt.wantStart, tt.wantEnd)
			}
		})
	}
}

func TestRangeSelectedTextClipsColumnsAndPreservesLineBreaks(t *testing.T) {
	lines := []string{"hello", "gode", "世界abc"}
	tests := []struct {
		name string
		rng  Range
		want string
	}{
		{
			name: "same line",
			rng:  Range{Anchor: Point{Line: 0, Column: 1}, Focus: Point{Line: 0, Column: 4}, Active: true},
			want: "ell",
		},
		{
			name: "multi line",
			rng:  Range{Anchor: Point{Line: 0, Column: 2}, Focus: Point{Line: 2, Column: 3}, Active: true},
			want: "llo\ngode\n世界a",
		},
		{
			name: "reverse multi line",
			rng:  Range{Anchor: Point{Line: 2, Column: 3}, Focus: Point{Line: 0, Column: 2}, Active: true},
			want: "llo\ngode\n世界a",
		},
		{
			name: "clips columns and lines",
			rng:  Range{Anchor: Point{Line: -4, Column: -3}, Focus: Point{Line: 9, Column: 99}, Active: true},
			want: "hello\ngode\n世界abc",
		},
		{
			name: "out of bounds collapsed after clipping",
			rng:  Range{Anchor: Point{Line: 9, Column: 1}, Focus: Point{Line: 10, Column: 2}, Active: true},
			want: "",
		},
		{
			name: "inactive",
			rng:  Range{Anchor: Point{Line: 0, Column: 0}, Focus: Point{Line: 0, Column: 5}},
			want: "",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := tt.rng.SelectedText(lines); got != tt.want {
				t.Fatalf("selected text = %q, want %q", got, tt.want)
			}
		})
	}
}

func TestRangeCharCountAndCanCopy(t *testing.T) {
	lines := []string{"ab", "世界"}
	tests := []struct {
		name      string
		rng       Range
		wantCount int
		wantCopy  bool
	}{
		{
			name:      "two runes is not copyable",
			rng:       Range{Anchor: Point{Line: 0, Column: 0}, Focus: Point{Line: 0, Column: 2}, Active: true},
			wantCount: 2,
			wantCopy:  false,
		},
		{
			name:      "three visible runes across newline is copyable",
			rng:       Range{Anchor: Point{Line: 0, Column: 0}, Focus: Point{Line: 1, Column: 1}, Active: true},
			wantCount: 3,
			wantCopy:  true,
		},
		{
			name:      "multi byte runes count as one visible rune each",
			rng:       Range{Anchor: Point{Line: 1, Column: 0}, Focus: Point{Line: 1, Column: 2}, Active: true},
			wantCount: 2,
			wantCopy:  false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := tt.rng.CharCount(lines); got != tt.wantCount {
				t.Fatalf("char count = %d, want %d", got, tt.wantCount)
			}
			if got := tt.rng.CanCopy(lines); got != tt.wantCopy {
				t.Fatalf("can copy = %v, want %v", got, tt.wantCopy)
			}
		})
	}
}
