package acp

import (
	"bufio"
	"context"
	"encoding/json"
	"io"
	"strings"
	"sync"
)

func (s *Server) ServeStdio(ctx context.Context, input io.Reader, output io.Writer) error {
	var writeMu sync.Mutex
	encoder := json.NewEncoder(output)
	conn := s.NewConnection(func(_ context.Context, msg Message) error {
		writeMu.Lock()
		defer writeMu.Unlock()
		return encoder.Encode(msg)
	})

	scanner := bufio.NewScanner(input)
	scanner.Buffer(make([]byte, 0, 64*1024), 8*1024*1024)
	for scanner.Scan() {
		select {
		case <-ctx.Done():
			return ctx.Err()
		default:
		}
		line := strings.TrimSpace(scanner.Text())
		if line == "" {
			continue
		}
		if err := conn.HandleJSON(ctx, []byte(line)); err != nil {
			return err
		}
	}
	if err := scanner.Err(); err != nil {
		return err
	}
	return nil
}
