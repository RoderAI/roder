package shell

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"

	"github.com/itchyny/gojq"
	"mvdan.cc/sh/v3/interp"
)

func RegisterJSONBuiltins(reg *BuiltinRegistry) error {
	return reg.Register(Builtin{
		Name:        "jq",
		Description: "run a small in-process jq-compatible JSON query",
		ReadOnly:    true,
		Run: func(ctx context.Context, args []string, stdin io.Reader, stdout io.Writer, stderr io.Writer) error {
			hc := interp.HandlerCtx(ctx)
			return handleJQInDir(ctx, args, stdin, stdout, stderr, hc.Dir)
		},
	})
}

func handleJQ(ctx context.Context, args []string, stdin io.Reader, stdout io.Writer, stderr io.Writer) error {
	return handleJQInDir(ctx, args, stdin, stdout, stderr, ".")
}

func handleJQInDir(ctx context.Context, args []string, stdin io.Reader, stdout io.Writer, stderr io.Writer, dir string) error {
	opts, err := parseJQArgs(args)
	if err != nil {
		fmt.Fprintln(stderr, err)
		return interp.ExitStatus(2)
	}
	query, err := gojq.Parse(opts.filter)
	if err != nil {
		fmt.Fprintf(stderr, "jq: invalid query: %v\n", err)
		return interp.ExitStatus(3)
	}
	inputs, err := jqInputs(ctx, stdin, dir, opts.files)
	if err != nil {
		if errors.Is(err, context.Canceled) || errors.Is(err, context.DeadlineExceeded) {
			return err
		}
		fmt.Fprintf(stderr, "jq: invalid json: %v\n", err)
		return interp.ExitStatus(4)
	}
	for _, input := range inputs {
		if err := ctx.Err(); err != nil {
			return err
		}
		iter := query.RunWithContext(ctx, input)
		for {
			if err := ctx.Err(); err != nil {
				return err
			}
			value, ok := iter.Next()
			if !ok {
				break
			}
			if err, ok := value.(error); ok {
				fmt.Fprintf(stderr, "jq: %v\n", err)
				return interp.ExitStatus(5)
			}
			if err := writeJQValue(stdout, value, opts.raw); err != nil {
				return err
			}
		}
	}
	return nil
}

type jqOptions struct {
	raw    bool
	filter string
	files  []string
}

func parseJQArgs(args []string) (jqOptions, error) {
	var opts jqOptions
	for _, arg := range args[1:] {
		if opts.filter == "" && arg == "-r" {
			opts.raw = true
			continue
		}
		if opts.filter == "" {
			opts.filter = arg
			continue
		}
		opts.files = append(opts.files, arg)
	}
	if opts.filter == "" {
		return opts, fmt.Errorf("jq: missing filter")
	}
	return opts, nil
}

func jqInputs(ctx context.Context, stdin io.Reader, dir string, files []string) ([]any, error) {
	if len(files) == 0 {
		if stdin == nil {
			return nil, nil
		}
		return decodeJSONStream(ctx, stdin)
	}
	var inputs []any
	for _, name := range files {
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		path := name
		if !filepath.IsAbs(path) {
			path = filepath.Join(dir, name)
		}
		file, err := os.Open(path)
		if err != nil {
			return nil, err
		}
		values, decodeErr := decodeJSONStream(ctx, file)
		closeErr := file.Close()
		if decodeErr != nil {
			return nil, decodeErr
		}
		if closeErr != nil {
			return nil, closeErr
		}
		inputs = append(inputs, values...)
	}
	return inputs, nil
}

func decodeJSONStream(ctx context.Context, reader io.Reader) ([]any, error) {
	decoder := json.NewDecoder(reader)
	decoder.UseNumber()
	var values []any
	for {
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		var value any
		err := decoder.Decode(&value)
		if errors.Is(err, io.EOF) {
			return values, nil
		}
		if err != nil {
			return nil, err
		}
		values = append(values, value)
	}
}

func writeJQValue(stdout io.Writer, value any, raw bool) error {
	if raw {
		if text, ok := value.(string); ok {
			_, err := fmt.Fprintln(stdout, text)
			return err
		}
	}
	data, err := gojq.Marshal(value)
	if err != nil {
		return err
	}
	_, err = stdout.Write(append(data, '\n'))
	return err
}
