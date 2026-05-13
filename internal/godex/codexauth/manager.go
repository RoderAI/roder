package codexauth

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
	"time"
)

const (
	ClientID              = "app_EMoamEEZ73f0CkXaXp7hrann"
	Issuer                = "https://auth.openai.com"
	CodexBaseURL          = "https://chatgpt.com/backend-api/codex"
	OAuthDummyKey         = "gode-oauth-dummy-key"
	CallbackPort          = 1455
	defaultAuthorizeURL   = "https://auth.openai.com/oauth/authorize"
	defaultTokenEndpoint  = "https://auth.openai.com/oauth/token"
	defaultTokenType      = "oauth"
	refreshExpirySkew     = 3 * time.Minute
	codeChallengeMethod   = "S256"
	authorizationCodeFlow = "code"
)

type PKCE struct {
	Verifier  string
	Challenge string
}

type TokenResponse struct {
	IDToken      string `json:"id_token"`
	AccessToken  string `json:"access_token"`
	RefreshToken string `json:"refresh_token"`
	ExpiresIn    int64  `json:"expires_in"`
	AccountID    string `json:"account_id,omitempty"`
	TokenType    string `json:"token_type,omitempty"`
}

type Manager struct {
	Store         Store
	HTTPClient    *http.Client
	TokenEndpoint string
	Now           func() time.Time
}

func AuthorizeURL(callbackURL string, pkce PKCE, state string) *url.URL {
	u, _ := url.Parse(defaultAuthorizeURL)
	q := u.Query()
	q.Set("response_type", authorizationCodeFlow)
	q.Set("client_id", ClientID)
	q.Set("redirect_uri", callbackURL)
	q.Set("scope", "openid profile email offline_access")
	q.Set("originator", "gode")
	q.Set("id_token_add_organizations", "true")
	q.Set("codex_cli_simplified_flow", "true")
	if pkce.Challenge != "" {
		q.Set("code_challenge", pkce.Challenge)
		q.Set("code_challenge_method", codeChallengeMethod)
	}
	if state != "" {
		q.Set("state", state)
	}
	u.RawQuery = q.Encode()
	return u
}

func GeneratePKCE() (PKCE, error) {
	verifier, err := randomString(43)
	if err != nil {
		return PKCE{}, err
	}
	sum := sha256.Sum256([]byte(verifier))
	return PKCE{
		Verifier:  verifier,
		Challenge: base64.RawURLEncoding.EncodeToString(sum[:]),
	}, nil
}

func GenerateState() (string, error) {
	return randomString(43)
}

func (m Manager) AccessToken(ctx context.Context) (string, string, error) {
	tokens, err := m.Store.Load()
	if err != nil {
		return "", "", err
	}
	now := m.now()
	if tokens.Access != "" && tokens.Expires > now.Add(refreshExpirySkew).UnixMilli() {
		return tokens.Access, tokens.AccountID, nil
	}
	if strings.TrimSpace(tokens.Refresh) == "" {
		return "", "", fmt.Errorf("codex auth is missing; run `gode auth login codex`")
	}

	refreshed, err := m.refresh(ctx, tokens.Refresh, now)
	if err != nil {
		return "", "", err
	}
	if refreshed.AccountID == "" {
		refreshed.AccountID = tokens.AccountID
	}
	if err := m.Store.Save(refreshed); err != nil {
		return "", "", err
	}
	return refreshed.Access, refreshed.AccountID, nil
}

func (m Manager) Refresh(ctx context.Context, refreshToken string) (TokenResponse, error) {
	form := url.Values{}
	form.Set("grant_type", "refresh_token")
	form.Set("refresh_token", refreshToken)
	form.Set("client_id", ClientID)
	return m.postToken(ctx, form)
}

func (m Manager) ExchangeCode(ctx context.Context, code string, redirectURI string, pkce PKCE) (TokenResponse, error) {
	form := url.Values{}
	form.Set("grant_type", "authorization_code")
	form.Set("code", code)
	form.Set("redirect_uri", redirectURI)
	form.Set("client_id", ClientID)
	form.Set("code_verifier", pkce.Verifier)
	return m.postToken(ctx, form)
}

func (m Manager) refresh(ctx context.Context, refreshToken string, now time.Time) (Tokens, error) {
	tokenResp, err := m.Refresh(ctx, refreshToken)
	if err != nil {
		return Tokens{}, err
	}
	tokens, err := tokensFromResponse(tokenResp, now)
	if err != nil {
		return Tokens{}, err
	}
	if tokens.Refresh == "" {
		tokens.Refresh = refreshToken
	}
	return tokens, nil
}

func tokensFromResponse(tokenResp TokenResponse, now time.Time) (Tokens, error) {
	if tokenResp.AccessToken == "" {
		return Tokens{}, fmt.Errorf("codex token response missing access_token")
	}
	expiresIn := tokenResp.ExpiresIn
	if expiresIn <= 0 {
		expiresIn = 3600
	}
	tokenType := tokenResp.TokenType
	if tokenType == "" {
		tokenType = defaultTokenType
	}
	accountID := ExtractAccountID(tokenResp)
	if accountID == "" {
		accountID = tokenResp.AccountID
	}
	return Tokens{
		Type:      tokenType,
		Access:    tokenResp.AccessToken,
		Refresh:   tokenResp.RefreshToken,
		Expires:   now.Add(time.Duration(expiresIn) * time.Second).UnixMilli(),
		AccountID: accountID,
	}, nil
}

func (m Manager) postToken(ctx context.Context, form url.Values) (TokenResponse, error) {
	endpoint := m.TokenEndpoint
	if endpoint == "" {
		endpoint = defaultTokenEndpoint
	}
	client := m.HTTPClient
	if client == nil {
		client = http.DefaultClient
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, endpoint, strings.NewReader(form.Encode()))
	if err != nil {
		return TokenResponse{}, err
	}
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	resp, err := client.Do(req)
	if err != nil {
		return TokenResponse{}, err
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		body, _ := io.ReadAll(io.LimitReader(resp.Body, 1024))
		return TokenResponse{}, fmt.Errorf("codex token request failed: %s %s", resp.Status, strings.TrimSpace(string(body)))
	}
	var tokenResp TokenResponse
	if err := json.NewDecoder(resp.Body).Decode(&tokenResp); err != nil {
		return TokenResponse{}, fmt.Errorf("decode codex token response: %w", err)
	}
	return tokenResp, nil
}

func (m Manager) now() time.Time {
	if m.Now != nil {
		return m.Now()
	}
	return time.Now()
}

func randomString(length int) (string, error) {
	const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~"
	buf := make([]byte, length)
	if _, err := rand.Read(buf); err != nil {
		return "", err
	}
	for i, b := range buf {
		buf[i] = alphabet[int(b)%len(alphabet)]
	}
	return string(buf), nil
}
