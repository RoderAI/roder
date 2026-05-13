package provider

import (
	"context"
	"fmt"
)

type placeholder struct {
	name string
}

func Placeholder(name string) Provider {
	return placeholder{name: name}
}

func (p placeholder) Name() string {
	return p.name
}

func (p placeholder) Complete(context.Context, []Message) (Response, error) {
	return Response{}, fmt.Errorf("%s provider is not implemented yet", p.name)
}
