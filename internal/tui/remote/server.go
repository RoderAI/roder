package remote

import (
	"context"
	"crypto/rand"
	"errors"
	"sync"
	"time"

	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/appserver"
)

type Controller struct {
	app *godex.App

	mu       sync.Mutex
	server   *appserver.Server
	listener *appserver.WebSocketListener
	token    string
	auth     appserver.RemoteAuth
	urls     []string
	qr       string
	started  time.Time
	err      string
}

type State struct {
	Running          bool
	URLs             []string
	TokenPreview     string
	QR               string
	AuthHeaderHint   string
	SubprotocolHint  string
	ConnectedClients int
	StartedAt        time.Time
	Error            string
}

func NewController(app *godex.App) *Controller {
	return &Controller{app: app}
}

func (c *Controller) Start(ctx context.Context) (State, error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.listener != nil {
		return c.stateLocked(), nil
	}
	if c.app == nil {
		c.err = "remote server requires an app"
		return c.stateLocked(), errors.New(c.err)
	}
	token, err := appserver.GenerateRemoteToken(rand.Reader)
	if err != nil {
		c.err = err.Error()
		return c.stateLocked(), err
	}
	auth, err := appserver.NewRemoteAuth(token.Token, time.Now())
	if err != nil {
		c.err = err.Error()
		return c.stateLocked(), err
	}
	server := appserver.New(c.app, appserver.Options{
		Version: "dev",
		Remote: appserver.RemoteOptions{
			Enabled:    true,
			Auth:       auth,
			ServerName: "Gode Remote",
		},
	})
	listener, err := server.ListenWebSocket(ctx, "0.0.0.0:0")
	if err != nil {
		c.err = err.Error()
		return c.stateLocked(), err
	}
	urls := appserver.DiscoverRemoteConnectURLs(listener.Address())
	if len(urls) == 0 {
		urls = []string{listener.WebSocketURL()}
	}
	payload := appserver.BuildRemotePairingPayload("Gode Remote", urls[0], token.Token, c.app.Config.Workspace)
	link, err := appserver.RemoteDeepLink(payload)
	if err != nil {
		_ = listener.Close(context.Background())
		c.err = err.Error()
		return c.stateLocked(), err
	}
	qr, err := appserver.RenderTerminalQR(link)
	if err != nil {
		_ = listener.Close(context.Background())
		c.err = err.Error()
		return c.stateLocked(), err
	}
	c.server = server
	c.listener = listener
	c.token = token.Token
	c.auth = auth
	c.urls = urls
	c.qr = qr
	c.started = time.Now()
	c.err = ""
	return c.stateLocked(), nil
}

func (c *Controller) Stop(ctx context.Context) (State, error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.listener == nil {
		return c.stateLocked(), nil
	}
	err := c.listener.Close(ctx)
	c.server = nil
	c.listener = nil
	c.token = ""
	c.auth = appserver.RemoteAuth{}
	c.urls = nil
	c.qr = ""
	c.started = time.Time{}
	if err != nil {
		c.err = err.Error()
		return c.stateLocked(), err
	}
	c.err = ""
	return c.stateLocked(), nil
}

func (c *Controller) Regenerate(ctx context.Context) (State, error) {
	if _, err := c.Stop(ctx); err != nil {
		return c.Snapshot(), err
	}
	return c.Start(ctx)
}

func (c *Controller) Snapshot() State {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.stateLocked()
}

func (c *Controller) stateLocked() State {
	state := State{
		Running:         c.listener != nil,
		URLs:            append([]string(nil), c.urls...),
		TokenPreview:    c.auth.TokenPreview,
		QR:              c.qr,
		AuthHeaderHint:  "Authorization: Bearer <token>",
		SubprotocolHint: "gode.remote.v1, bearer.<token>",
		StartedAt:       c.started,
		Error:           c.err,
	}
	if c.server != nil {
		state.ConnectedClients = c.server.ConnectionCount()
	}
	return state
}
