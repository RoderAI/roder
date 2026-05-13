package appserver

import (
	"context"
	"sync"

	"github.com/pandelisz/gode/internal/godex"
)

type SendFunc func(context.Context, Message) error

type Server struct {
	app     *godex.App
	options Options

	mu       sync.RWMutex
	threads  map[string]*threadState
	commands map[string]*activeCommand
	conns    map[*Connection]struct{}
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
		conns:    make(map[*Connection]struct{}),
	}
	server.startEventBridge(context.Background())
	return server
}
