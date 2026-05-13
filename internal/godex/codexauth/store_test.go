package codexauth

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"
)

func TestManagerRefreshesExpiredTokensAndPersists(t *testing.T) {
	dataDir := t.TempDir()
	now := time.Unix(100, 0)
	store := Store{DataDir: dataDir}
	if err := store.Save(Tokens{
		Access:  "old-access",
		Refresh: "old-refresh",
		Expires: now.Add(-time.Minute).UnixMilli(),
	}); err != nil {
		t.Fatalf("save: %v", err)
	}

	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/oauth/token" {
			t.Fatalf("path = %q", r.URL.Path)
		}
		if err := r.ParseForm(); err != nil {
			t.Fatalf("parse form: %v", err)
		}
		if r.Form.Get("grant_type") != "refresh_token" {
			t.Fatalf("grant_type = %q", r.Form.Get("grant_type"))
		}
		if r.Form.Get("refresh_token") != "old-refresh" {
			t.Fatalf("refresh_token = %q", r.Form.Get("refresh_token"))
		}
		_ = json.NewEncoder(w).Encode(TokenResponse{
			AccessToken:  "new-access",
			RefreshToken: "new-refresh",
			ExpiresIn:    3600,
		})
	}))
	defer server.Close()

	manager := Manager{
		Store:         store,
		HTTPClient:    server.Client(),
		TokenEndpoint: server.URL + "/oauth/token",
		Now:           func() time.Time { return now },
	}
	access, _, err := manager.AccessToken(context.Background())
	if err != nil {
		t.Fatalf("access token: %v", err)
	}
	if access != "new-access" {
		t.Fatalf("access = %q", access)
	}

	got, err := store.Load()
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	if got.Access != "new-access" || got.Refresh != "new-refresh" {
		t.Fatalf("stored tokens = %#v", got)
	}
	if got.Expires <= now.UnixMilli() {
		t.Fatalf("expires = %d, want future", got.Expires)
	}
}

func TestAuthorizeURLUsesGodeOriginator(t *testing.T) {
	u := AuthorizeURL("http://localhost:1455/auth/callback", PKCE{Challenge: "challenge"}, "state")
	if got := u.Query().Get("originator"); got != "gode" {
		t.Fatalf("originator = %q", got)
	}
	if got := u.Query().Get("client_id"); got != ClientID {
		t.Fatalf("client_id = %q", got)
	}
}
