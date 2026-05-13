package memory

import (
	"encoding/binary"
	"fmt"
	"math"
)

func EncodeVector(vector Vector) ([]byte, error) {
	dims := vector.Dimensions
	if dims == 0 {
		dims = len(vector.Values)
	}
	if dims != len(vector.Values) {
		return nil, fmt.Errorf("vector dimensions %d do not match values %d", dims, len(vector.Values))
	}
	blob := make([]byte, len(vector.Values)*4)
	for i, value := range vector.Values {
		binary.LittleEndian.PutUint32(blob[i*4:], math.Float32bits(value))
	}
	return blob, nil
}

func DecodeVector(model string, dimensions int, blob []byte) (Vector, error) {
	if len(blob)%4 != 0 {
		return Vector{}, fmt.Errorf("vector blob length %d is not divisible by 4", len(blob))
	}
	values := make([]float32, len(blob)/4)
	for i := range values {
		values[i] = math.Float32frombits(binary.LittleEndian.Uint32(blob[i*4:]))
	}
	if dimensions != 0 && dimensions != len(values) {
		return Vector{}, fmt.Errorf("vector dimensions %d do not match values %d", dimensions, len(values))
	}
	if dimensions == 0 {
		dimensions = len(values)
	}
	return Vector{Model: model, Dimensions: dimensions, Values: values}, nil
}

func Similarity(a Vector, b Vector) (float64, error) {
	if len(a.Values) == 0 || len(b.Values) == 0 {
		if len(a.Values) != len(b.Values) {
			return 0, fmt.Errorf("vector dimension mismatch: %d != %d", len(a.Values), len(b.Values))
		}
		return 0, nil
	}
	if len(a.Values) != len(b.Values) {
		return 0, fmt.Errorf("vector dimension mismatch: %d != %d", len(a.Values), len(b.Values))
	}
	var dot, normA, normB float64
	for i := range a.Values {
		av := float64(a.Values[i])
		bv := float64(b.Values[i])
		dot += av * bv
		normA += av * av
		normB += bv * bv
	}
	if normA == 0 || normB == 0 {
		return 0, nil
	}
	return dot / (math.Sqrt(normA) * math.Sqrt(normB)), nil
}
