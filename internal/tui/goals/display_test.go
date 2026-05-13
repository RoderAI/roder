package goals

import "testing"

func TestElapsed(t *testing.T) {
	tests := map[int64]string{
		0:     "0s",
		59:    "59s",
		60:    "1m",
		5400:  "1h 30m",
		86400: "1d 0h 0m",
	}
	for seconds, want := range tests {
		if got := Elapsed(seconds); got != want {
			t.Fatalf("Elapsed(%d) = %q, want %q", seconds, got, want)
		}
	}
}

func TestBudget(t *testing.T) {
	budget := int64(10000)
	if got := Budget(3300, &budget); got != "3.3K/10K" {
		t.Fatalf("budget = %q", got)
	}
}
