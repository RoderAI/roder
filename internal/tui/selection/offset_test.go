package selection

import "testing"

func TestOffsetRangeNormalize(t *testing.T) {
	text := "ab世界cd"
	tests := []struct {
		name      string
		rng       OffsetRange
		wantStart int
		wantEnd   int
		wantOK    bool
	}{
		{
			name:      "forward",
			rng:       OffsetRange{Anchor: 1, Focus: 4, Active: true},
			wantStart: 1,
			wantEnd:   4,
			wantOK:    true,
		},
		{
			name:      "reverse",
			rng:       OffsetRange{Anchor: 4, Focus: 1, Active: true},
			wantStart: 1,
			wantEnd:   4,
			wantOK:    true,
		},
		{
			name:   "collapsed",
			rng:    OffsetRange{Anchor: 3, Focus: 3, Active: true},
			wantOK: false,
		},
		{
			name:   "inactive",
			rng:    OffsetRange{Anchor: 1, Focus: 3},
			wantOK: false,
		},
		{
			name:      "out of bounds clamps",
			rng:       OffsetRange{Anchor: -20, Focus: 200, Active: true},
			wantStart: 0,
			wantEnd:   6,
			wantOK:    true,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			start, end, ok := tt.rng.Normalize(text)
			if ok != tt.wantOK {
				t.Fatalf("ok = %v, want %v", ok, tt.wantOK)
			}
			if start != tt.wantStart || end != tt.wantEnd {
				t.Fatalf("range = %d..%d, want %d..%d", start, end, tt.wantStart, tt.wantEnd)
			}
		})
	}
}

func TestOffsetRangeSelectedText(t *testing.T) {
	text := "ab世界cd"
	tests := []struct {
		name string
		rng  OffsetRange
		want string
	}{
		{
			name: "forward multi rune",
			rng:  OffsetRange{Anchor: 1, Focus: 4, Active: true},
			want: "b世界",
		},
		{
			name: "reverse multi rune",
			rng:  OffsetRange{Anchor: 4, Focus: 1, Active: true},
			want: "b世界",
		},
		{
			name: "out of bounds",
			rng:  OffsetRange{Anchor: -5, Focus: 99, Active: true},
			want: text,
		},
		{
			name: "collapsed",
			rng:  OffsetRange{Anchor: 2, Focus: 2, Active: true},
			want: "",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := tt.rng.SelectedText(text); got != tt.want {
				t.Fatalf("selected text = %q, want %q", got, tt.want)
			}
		})
	}
}

func TestOffsetRangeCharCountAndCanCopy(t *testing.T) {
	text := "ab世界cd"
	tests := []struct {
		name      string
		rng       OffsetRange
		wantCount int
		wantCopy  bool
	}{
		{
			name:      "short range",
			rng:       OffsetRange{Anchor: 0, Focus: 2, Active: true},
			wantCount: 2,
			wantCopy:  false,
		},
		{
			name:      "three rune range",
			rng:       OffsetRange{Anchor: 0, Focus: 3, Active: true},
			wantCount: 3,
			wantCopy:  true,
		},
		{
			name:      "multi byte runes count as visible runes",
			rng:       OffsetRange{Anchor: 2, Focus: 4, Active: true},
			wantCount: 2,
			wantCopy:  false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := tt.rng.CharCount(text); got != tt.wantCount {
				t.Fatalf("char count = %d, want %d", got, tt.wantCount)
			}
			if got := tt.rng.CanCopy(text); got != tt.wantCopy {
				t.Fatalf("can copy = %v, want %v", got, tt.wantCopy)
			}
		})
	}
}
