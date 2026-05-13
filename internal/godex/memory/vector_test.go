package memory

import (
	"math"
	"testing"
)

func TestVectorEncodeDecodeLittleEndian(t *testing.T) {
	vector := Vector{Model: "test-embed", Dimensions: 3, Values: []float32{1.5, -2, 0.25}}
	blob, err := EncodeVector(vector)
	if err != nil {
		t.Fatalf("encode: %v", err)
	}
	if len(blob) != 12 {
		t.Fatalf("blob length = %d", len(blob))
	}

	decoded, err := DecodeVector("test-embed", 3, blob)
	if err != nil {
		t.Fatalf("decode: %v", err)
	}
	if decoded.Model != vector.Model || decoded.Dimensions != vector.Dimensions || len(decoded.Values) != len(vector.Values) {
		t.Fatalf("decoded = %#v", decoded)
	}
	for i := range vector.Values {
		if decoded.Values[i] != vector.Values[i] {
			t.Fatalf("value %d = %v, want %v", i, decoded.Values[i], vector.Values[i])
		}
	}
}

func TestVectorSimilarity(t *testing.T) {
	tests := []struct {
		name string
		a    Vector
		b    Vector
		want float64
	}{
		{
			name: "exact",
			a:    Vector{Values: []float32{1, 1}},
			b:    Vector{Values: []float32{1, 1}},
			want: 1,
		},
		{
			name: "orthogonal",
			a:    Vector{Values: []float32{1, 0}},
			b:    Vector{Values: []float32{0, 1}},
			want: 0,
		},
		{
			name: "empty",
			a:    Vector{},
			b:    Vector{},
			want: 0,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := Similarity(tt.a, tt.b)
			if err != nil {
				t.Fatalf("similarity: %v", err)
			}
			if math.Abs(got-tt.want) > 0.00001 {
				t.Fatalf("similarity = %v, want %v", got, tt.want)
			}
		})
	}
}

func TestVectorSimilarityDimensionMismatch(t *testing.T) {
	if _, err := Similarity(Vector{Values: []float32{1}}, Vector{Values: []float32{1, 2}}); err == nil {
		t.Fatal("expected dimension mismatch")
	}
}
