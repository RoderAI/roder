package appserver

import (
	"context"
	"sync"
	"time"

	"github.com/pandelisz/gode/internal/godex"
)

type SendFunc func(context.Context, Message) error

type Server struct {
	app     *godex.App
	options Options

	mu                sync.RWMutex
	threads           map[string]*threadState
	commands          map[string]*activeCommand
	uploads           map[string]*activeUpload
	conns             map[*Connection]struct{}
	remoteAuthBackoff RemoteAuthBackoff
}

type Connection struct {
	server   *Server
	sendFunc SendFunc

	mu          sync.RWMutex
	sendMu      sync.Mutex
	initialized bool
	clientInfo  ClientInfo
	optOut      map[string]struct{}
	subscribed  map[string]struct{}
}

func (s *Server) recordRemoteAuthFailure() time.Duration {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.remoteAuthBackoff.RecordFailure()
}

func (s *Server) resetRemoteAuthBackoff() {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.remoteAuthBackoff.Reset()
}

type threadState struct {
	Thread
	activeCancel context.CancelFunc
	activeTurnID string
}

func New(app *godex.App, options Options) *Server {
	if options.Version == "" {
		options.Version = "dev"
	}
	server := &Server{
		app:      app,
		options:  options,
		threads:  make(map[string]*threadState),
		commands: make(map[string]*activeCommand),
		uploads:  make(map[string]*activeUpload),
		conns:    make(map[*Connection]struct{}),
	}
	server.startEventBridge(context.Background())
	return server
}

func (s *Server) ConnectionCount() int {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return len(s.conns)
}
