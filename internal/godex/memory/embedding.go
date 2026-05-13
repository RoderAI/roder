package memory

import (
	"context"
	"errors"
	"fmt"
	"strings"

	openai "github.com/openai/openai-go/v3"
	"github.com/openai/openai-go/v3/option"
	"github.com/openai/openai-go/v3/packages/param"
)

type Embedder interface {
	Embed(ctx context.Context, input string) (Vector, error)
	Model() string
}

type embeddingClient interface {
	New(context.Context, openai.EmbeddingNewParams, ...option.RequestOption) (*openai.CreateEmbeddingResponse, error)
}

type OpenAIEmbedder struct {
	client embeddingClient
	model  string
}

func NewOpenAIEmbedder(model string, opts ...option.RequestOption) *OpenAIEmbedder {
	if strings.TrimSpace(model) == "" {
		model = DefaultEmbeddingModel
	}
	client := openai.NewClient(opts...)
	return newOpenAIEmbedderWithClient(model, &client.Embeddings)
}

func newOpenAIEmbedderWithClient(model string, client embeddingClient) *OpenAIEmbedder {
	if strings.TrimSpace(model) == "" {
		model = DefaultEmbeddingModel
	}
	return &OpenAIEmbedder{client: client, model: model}
}

func (e *OpenAIEmbedder) Model() string {
	if e == nil || strings.TrimSpace(e.model) == "" {
		return DefaultEmbeddingModel
	}
	return e.model
}

func (e *OpenAIEmbedder) Embed(ctx context.Context, input string) (Vector, error) {
	input = strings.TrimSpace(input)
	if input == "" {
		return Vector{}, errors.New("embedding input is required")
	}
	if e == nil || e.client == nil {
		return Vector{}, errors.New("embedding client is required")
	}
	model := e.Model()
	resp, err := e.client.New(ctx, openai.EmbeddingNewParams{
		Input: openai.EmbeddingNewParamsInputUnion{
			OfString: param.NewOpt(input),
		},
		Model:          openai.EmbeddingModel(model),
		EncodingFormat: openai.EmbeddingNewParamsEncodingFormatFloat,
	})
	if err != nil {
		return Vector{}, fmt.Errorf("embedding failed with %s: %w", model, err)
	}
	if resp == nil || len(resp.Data) == 0 {
		return Vector{}, fmt.Errorf("embedding failed with %s: empty response", model)
	}
	values := make([]float32, len(resp.Data[0].Embedding))
	for i, value := range resp.Data[0].Embedding {
		values[i] = float32(value)
	}
	if strings.TrimSpace(resp.Model) != "" {
		model = resp.Model
	}
	return Vector{Model: model, Dimensions: len(values), Values: values}, nil
}
