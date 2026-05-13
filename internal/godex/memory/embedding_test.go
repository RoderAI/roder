package memory

import (
	"context"
	"errors"
	"os"
	"reflect"
	"strings"
	"testing"

	openai "github.com/openai/openai-go/v3"
	"github.com/openai/openai-go/v3/option"
)

type fakeEmbeddingClient struct {
	calls  int
	err    error
	result *openai.CreateEmbeddingResponse
}

func (f *fakeEmbeddingClient) New(ctx context.Context, params openai.EmbeddingNewParams, opts ...option.RequestOption) (*openai.CreateEmbeddingResponse, error) {
	f.calls++
	if f.err != nil {
		return nil, f.err
	}
	return f.result, nil
}

type deterministicFakeEmbedder struct {
	model string
}

func (d deterministicFakeEmbedder) Model() string {
	return d.model
}

func (d deterministicFakeEmbedder) Embed(ctx context.Context, input string) (Vector, error) {
	input = strings.TrimSpace(input)
	if input == "" {
		return Vector{}, errors.New("input is required")
	}
	var values [3]float32
	for i, r := range input {
		values[i%3] += float32((int(r) % 97) + 1)
	}
	return Vector{Model: d.model, Dimensions: 3, Values: values[:]}, nil
}

func TestFakeEmbedderReturnsDeterministicThreeDimensionalVectors(t *testing.T) {
	embedder := deterministicFakeEmbedder{model: "fake-embedding"}

	first, err := embedder.Embed(context.Background(), "same input")
	if err != nil {
		t.Fatalf("embed first: %v", err)
	}
	second, err := embedder.Embed(context.Background(), "same input")
	if err != nil {
		t.Fatalf("embed second: %v", err)
	}
	different, err := embedder.Embed(context.Background(), "different input")
	if err != nil {
		t.Fatalf("embed different: %v", err)
	}

	if first.Model != "fake-embedding" || first.Dimensions != 3 || len(first.Values) != 3 {
		t.Fatalf("first = %#v", first)
	}
	if !reflect.DeepEqual(first.Values, second.Values) {
		t.Fatalf("same input was not deterministic: %#v != %#v", first.Values, second.Values)
	}
	if reflect.DeepEqual(first.Values, different.Values) {
		t.Fatalf("different inputs produced identical vectors: %#v", first.Values)
	}
}

func TestOpenAIEmbedderRejectsEmptyInputBeforeAPICall(t *testing.T) {
	client := &fakeEmbeddingClient{}
	embedder := newOpenAIEmbedderWithClient(DefaultEmbeddingModel, client)

	_, err := embedder.Embed(context.Background(), " \n\t ")
	if err == nil || !strings.Contains(err.Error(), "input is required") {
		t.Fatalf("err = %v", err)
	}
	if client.calls != 0 {
		t.Fatalf("client calls = %d", client.calls)
	}
}

func TestOpenAIEmbedderRecordsReturnedDimensions(t *testing.T) {
	client := &fakeEmbeddingClient{result: &openai.CreateEmbeddingResponse{
		Model: DefaultEmbeddingModel,
		Data:  []openai.Embedding{{Embedding: []float64{0.25, 0.5, 0.75}}},
	}}
	embedder := newOpenAIEmbedderWithClient(DefaultEmbeddingModel, client)

	vector, err := embedder.Embed(context.Background(), "hello")
	if err != nil {
		t.Fatalf("embed: %v", err)
	}
	if vector.Model != DefaultEmbeddingModel || vector.Dimensions != 3 {
		t.Fatalf("vector = %#v", vector)
	}
	if got := vector.Values; !reflect.DeepEqual(got, []float32{0.25, 0.5, 0.75}) {
		t.Fatalf("values = %#v", got)
	}
	if client.calls != 1 {
		t.Fatalf("client calls = %d", client.calls)
	}
}

func TestOpenAIEmbedderFailureIncludesModel(t *testing.T) {
	client := &fakeEmbeddingClient{err: errors.New("transport unavailable")}
	embedder := newOpenAIEmbedderWithClient(DefaultEmbeddingModel, client)

	_, err := embedder.Embed(context.Background(), "hello")
	if err == nil {
		t.Fatal("expected error")
	}
	if got := err.Error(); !strings.Contains(got, DefaultEmbeddingModel) || !strings.Contains(got, "embedding failed") {
		t.Fatalf("err = %v", err)
	}
}

func TestOpenAIEmbedderLive(t *testing.T) {
	if testing.Short() || strings.TrimSpace(os.Getenv("GODE_LIVE_EMBEDDINGS")) != "1" || strings.TrimSpace(os.Getenv("OPENAI_API_KEY")) == "" {
		t.Skip("set GODE_LIVE_EMBEDDINGS=1 and OPENAI_API_KEY to run")
	}
	embedder := NewOpenAIEmbedder(DefaultEmbeddingModel)
	vector, err := embedder.Embed(context.Background(), "gode live embedding smoke test")
	if err != nil {
		t.Fatalf("live embed: %v", err)
	}
	if vector.Model != DefaultEmbeddingModel || vector.Dimensions <= 0 || len(vector.Values) != vector.Dimensions {
		t.Fatalf("vector = %#v", vector)
	}
}
