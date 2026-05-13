package codexauth

import (
	"context"
	"fmt"
	"html"
	"net"
	"net/http"
	"os/exec"
	"runtime"
	"time"
)

const callbackPath = "/auth/callback"

func LoginBrowser(ctx context.Context, dataDir string) (Tokens, string, error) {
	pkce, err := GeneratePKCE()
	if err != nil {
		return Tokens{}, "", err
	}
	state, err := GenerateState()
	if err != nil {
		return Tokens{}, "", err
	}

	redirectURI := fmt.Sprintf("http://localhost:%d%s", CallbackPort, callbackPath)
	result := make(chan loginResult, 1)
	server := &http.Server{Handler: callbackHandler(ctx, Store{DataDir: dataDir}, redirectURI, pkce, state, result)}
	listener, err := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", CallbackPort))
	if err != nil {
		return Tokens{}, "", fmt.Errorf("start codex callback server: %w", err)
	}
	defer listener.Close()
	go func() {
		if err := server.Serve(listener); err != nil && err != http.ErrServerClosed {
			result <- loginResult{err: err}
		}
	}()
	defer server.Shutdown(context.Background())

	authURL := AuthorizeURL(redirectURI, pkce, state).String()
	if err := openBrowser(authURL); err != nil {
		return Tokens{}, authURL, err
	}

	select {
	case got := <-result:
		return got.tokens, authURL, got.err
	case <-time.After(5 * time.Minute):
		return Tokens{}, authURL, fmt.Errorf("codex login timed out")
	case <-ctx.Done():
		return Tokens{}, authURL, ctx.Err()
	}
}

type loginResult struct {
	tokens Tokens
	err    error
}

func callbackHandler(ctx context.Context, store Store, redirectURI string, pkce PKCE, state string, result chan<- loginResult) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != callbackPath {
			http.NotFound(w, r)
			return
		}
		if errText := r.URL.Query().Get("error"); errText != "" {
			sendCallbackHTML(w, false, errText)
			result <- loginResult{err: fmt.Errorf("codex login failed: %s", errText)}
			return
		}
		if got := r.URL.Query().Get("state"); got != state {
			sendCallbackHTML(w, false, "Invalid OAuth state")
			result <- loginResult{err: fmt.Errorf("invalid oauth state")}
			return
		}
		code := r.URL.Query().Get("code")
		if code == "" {
			sendCallbackHTML(w, false, "Missing authorization code")
			result <- loginResult{err: fmt.Errorf("missing authorization code")}
			return
		}
		resp, err := (Manager{Store: store}).ExchangeCode(ctx, code, redirectURI, pkce)
		if err != nil {
			sendCallbackHTML(w, false, err.Error())
			result <- loginResult{err: err}
			return
		}
		tokens, err := tokensFromResponse(resp, time.Now())
		if err != nil {
			sendCallbackHTML(w, false, err.Error())
			result <- loginResult{err: err}
			return
		}
		if err := store.Save(tokens); err != nil {
			sendCallbackHTML(w, false, err.Error())
			result <- loginResult{err: err}
			return
		}
		sendCallbackHTML(w, true, "")
		result <- loginResult{tokens: tokens}
	})
}

func sendCallbackHTML(w http.ResponseWriter, ok bool, message string) {
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	status := "Connected to Codex"
	body := "gode can now use your ChatGPT Codex session. You can close this tab."
	accent := "#73f7c5"
	if !ok {
		status = "Codex sign-in failed"
		body = html.EscapeString(message)
		accent = "#ff7a90"
	}
	fmt.Fprintf(w, callbackHTML, accent, status, body)
}

func openBrowser(url string) error {
	switch runtime.GOOS {
	case "darwin":
		return exec.Command("open", url).Start()
	case "windows":
		return exec.Command("rundll32", "url.dll,FileProtocolHandler", url).Start()
	default:
		return exec.Command("xdg-open", url).Start()
	}
}

const callbackHTML = `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>gode Codex sign-in</title>
  <style>
    :root { color-scheme: dark; font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }
    body { margin: 0; min-height: 100vh; display: grid; place-items: center; background: radial-gradient(circle at 20%% 20%%, #26354f 0, transparent 36%%), linear-gradient(135deg, #080b12, #171820 54%%, #101720); color: #f5f7fb; }
    main { width: min(560px, calc(100vw - 48px)); border: 1px solid rgba(255,255,255,.14); border-radius: 24px; padding: 36px; background: rgba(13,17,25,.78); box-shadow: 0 24px 80px rgba(0,0,0,.45); backdrop-filter: blur(16px); }
    .mark { width: 52px; height: 52px; display: grid; place-items: center; border-radius: 16px; background: %s; color: #081018; font-weight: 900; font-size: 24px; margin-bottom: 24px; }
    h1 { margin: 0 0 12px; font-size: clamp(30px, 5vw, 48px); line-height: 1; letter-spacing: 0; }
    p { margin: 0; color: #b8c1d1; font-size: 17px; line-height: 1.6; }
    .footer { margin-top: 28px; color: #7f8ba0; font-size: 13px; }
  </style>
</head>
<body>
  <main>
    <div class="mark">g</div>
    <h1>%s</h1>
    <p>%s</p>
    <div class="footer">gode Codex auth</div>
  </main>
  <script>setTimeout(() => window.close(), 1800)</script>
</body>
</html>`
