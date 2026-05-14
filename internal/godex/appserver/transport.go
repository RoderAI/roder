package appserver

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"strings"
	"sync"

	"nhooyr.io/websocket"
)

type TransportKind int

const (
	TransportStdio TransportKind = iota
	TransportWebSocket
	TransportOff
)

type ListenConfig struct {
	Kind    TransportKind
	Address string
	Remote  RemoteListenConfig
}

func ParseListenURL(raw string) (ListenConfig, error) {
	switch {
	case raw == "" || raw == "stdio://":
		return ListenConfig{Kind: TransportStdio}, nil
	case raw == "off":
		return ListenConfig{Kind: TransportOff}, nil
	case strings.HasPrefix(raw, "ws://"):
		address := strings.TrimPrefix(raw, "ws://")
		if address == "" {
			return ListenConfig{}, fmt.Errorf("invalid websocket listen URL %q", raw)
		}
		if _, err := net.ResolveTCPAddr("tcp", address); err != nil {
			return ListenConfig{}, fmt.Errorf("invalid websocket listen URL %q: %w", raw, err)
		}
		return ListenConfig{Kind: TransportWebSocket, Address: address}, nil
	default:
		return ListenConfig{}, fmt.Errorf("unsupported listen URL %q; expected stdio://, ws://IP:PORT, or off", raw)
	}
}

func (s *Server) ServeStdio(ctx context.Context, input io.Reader, output io.Writer) error {
	var writeMu sync.Mutex
	encoder := json.NewEncoder(output)
	conn := s.NewConnection(func(_ context.Context, msg Message) error {
		writeMu.Lock()
		defer writeMu.Unlock()
		return encoder.Encode(msg)
	})
	defer conn.Close()

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

type WebSocketListener struct {
	server   *http.Server
	listener net.Listener
	httpURL  string
	wsURL    string
}

func (s *Server) ListenWebSocket(ctx context.Context, address string) (*WebSocketListener, error) {
	listener, err := net.Listen("tcp", address)
	if err != nil {
		return nil, err
	}
	actual := listener.Addr().String()
	wsListener := &WebSocketListener{
		listener: listener,
		httpURL:  "http://" + actual,
		wsURL:    "ws://" + actual,
	}
	mux := http.NewServeMux()
	mux.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) {
		if !s.allowOrigin(r.Header.Get("Origin")) {
			http.Error(w, "Origin header is not allowed", http.StatusForbidden)
			return
		}
		switch r.URL.Path {
		case "/readyz", "/healthz":
			w.WriteHeader(http.StatusOK)
			_, _ = w.Write([]byte("OK"))
			return
		default:
			s.handleWebSocket(ctx, w, r)
		}
	})
	wsListener.server = &http.Server{Handler: mux}
	go func() {
		_ = wsListener.server.Serve(listener)
	}()
	return wsListener, nil
}

func (l *WebSocketListener) HTTPURL() string {
	return l.httpURL
}

func (l *WebSocketListener) WebSocketURL() string {
	return l.wsURL
}

func (l *WebSocketListener) Address() string {
	if l == nil || l.listener == nil {
		return ""
	}
	return l.listener.Addr().String()
}

func (l *WebSocketListener) Close(ctx context.Context) error {
	if l == nil || l.server == nil {
		return nil
	}
	return l.server.Shutdown(ctx)
}

func (s *Server) handleWebSocket(ctx context.Context, w http.ResponseWriter, r *http.Request) {
	if s.options.Remote.Enabled && !s.options.Remote.Auth.VerifyRequest(r) {
		http.Error(w, "remote authentication required", http.StatusUnauthorized)
		return
	}
	acceptOptions := &websocket.AcceptOptions{InsecureSkipVerify: true}
	if s.options.Remote.Enabled {
		acceptOptions.Subprotocols = []string{remoteSubprotocol}
	}
	ws, err := websocket.Accept(w, r, acceptOptions)
	if err != nil {
		return
	}
	conn := s.NewConnection(func(sendCtx context.Context, msg Message) error {
		data, err := json.Marshal(msg)
		if err != nil {
			return err
		}
		return ws.Write(sendCtx, websocket.MessageText, data)
	})
	defer conn.Close()
	defer ws.Close(websocket.StatusNormalClosure, "")

	for {
		typ, data, err := ws.Read(ctx)
		if err != nil {
			return
		}
		if typ != websocket.MessageText {
			continue
		}
		if err := conn.HandleJSON(ctx, data); err != nil {
			return
		}
	}
}

func (s *Server) allowOrigin(origin string) bool {
	if origin == "" {
		return true
	}
	for _, allowed := range s.options.Remote.AllowedOrigins {
		if strings.EqualFold(strings.TrimSpace(allowed), origin) {
			return true
		}
	}
	return false
}
