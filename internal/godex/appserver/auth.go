package appserver

import (
	"crypto/sha256"
	"crypto/subtle"
	"encoding/base64"
	"fmt"
	"io"
	"net/http"
	"strings"
	"time"
)

const (
	remoteTokenBytes  = 32
	remoteSubprotocol = "gode.remote.v1"
)

type RemoteToken struct {
	Token   string
	Preview string
	Hash    []byte
}

type RemoteAuth struct {
	Enabled       bool
	TokenHash     []byte
	TokenPreview  string
	HeaderName    string
	AllowInsecure bool
	CreatedAt     time.Time
	ExpiresAt     *time.Time
	Now           func() time.Time
}

func GenerateRemoteToken(rand io.Reader) (RemoteToken, error) {
	material := make([]byte, remoteTokenBytes)
	if _, err := io.ReadFull(rand, material); err != nil {
		return RemoteToken{}, fmt.Errorf("generate remote token: %w", err)
	}
	token := base64.RawURLEncoding.EncodeToString(material)
	return remoteTokenFromString(token)
}

func NewRemoteAuth(token string, now time.Time) (RemoteAuth, error) {
	remoteToken, err := remoteTokenFromString(token)
	if err != nil {
		return RemoteAuth{}, err
	}
	return RemoteAuth{
		Enabled:      true,
		TokenHash:    remoteToken.Hash,
		TokenPreview: remoteToken.Preview,
		HeaderName:   "Authorization",
		CreatedAt:    now,
	}, nil
}

func (a RemoteAuth) VerifyRequest(r *http.Request) bool {
	if !a.Enabled {
		return true
	}
	if len(a.TokenHash) == 0 {
		return false
	}
	if a.ExpiresAt != nil && a.now().After(*a.ExpiresAt) {
		return false
	}
	for _, candidate := range a.requestTokens(r) {
		hash := hashRemoteToken(candidate)
		if subtle.ConstantTimeCompare(a.TokenHash, hash) == 1 {
			return true
		}
	}
	return false
}

func (a RemoteAuth) now() time.Time {
	if a.Now != nil {
		return a.Now()
	}
	return time.Now()
}

type RemoteAuthBackoff struct {
	Failures    int
	LastFailure time.Time
	BaseDelay   time.Duration
	MaxDelay    time.Duration
	Now         func() time.Time
}

func (b *RemoteAuthBackoff) RecordFailure() time.Duration {
	if b == nil {
		return 0
	}
	b.Failures++
	b.LastFailure = b.now()
	return b.Delay()
}

func (b *RemoteAuthBackoff) Reset() {
	if b == nil {
		return
	}
	b.Failures = 0
	b.LastFailure = time.Time{}
}

func (b RemoteAuthBackoff) Delay() time.Duration {
	if b.Failures <= 0 {
		return 0
	}
	base := b.BaseDelay
	if base <= 0 {
		base = 250 * time.Millisecond
	}
	maxDelay := b.MaxDelay
	if maxDelay <= 0 {
		maxDelay = 5 * time.Second
	}
	delay := base
	for i := 1; i < b.Failures; i++ {
		delay *= 2
		if delay >= maxDelay {
			return maxDelay
		}
	}
	return delay
}

func (b RemoteAuthBackoff) now() time.Time {
	if b.Now != nil {
		return b.Now()
	}
	return time.Now()
}

func (a RemoteAuth) requestTokens(r *http.Request) []string {
	headerName := strings.TrimSpace(a.HeaderName)
	if headerName == "" {
		headerName = "Authorization"
	}
	var out []string
	if token, ok := bearerToken(r.Header.Get(headerName)); ok {
		out = append(out, token)
	}
	for _, value := range r.Header.Values("Sec-WebSocket-Protocol") {
		for _, part := range strings.Split(value, ",") {
			part = strings.TrimSpace(part)
			if strings.HasPrefix(part, "bearer.") {
				token := strings.TrimPrefix(part, "bearer.")
				if token != "" {
					out = append(out, token)
				}
			}
		}
	}
	return out
}

func bearerToken(value string) (string, bool) {
	value = strings.TrimSpace(value)
	if len(value) < len("Bearer ")+1 {
		return "", false
	}
	if !strings.EqualFold(value[:len("Bearer")], "Bearer") {
		return "", false
	}
	token := strings.TrimSpace(value[len("Bearer"):])
	if token == "" {
		return "", false
	}
	return token, true
}

func remoteTokenFromString(token string) (RemoteToken, error) {
	token = strings.TrimSpace(token)
	if token == "" {
		return RemoteToken{}, fmt.Errorf("remote auth token is empty")
	}
	hash := hashRemoteToken(token)
	return RemoteToken{Token: token, Preview: tokenPreview(token), Hash: hash}, nil
}

func hashRemoteToken(token string) []byte {
	sum := sha256.Sum256([]byte(token))
	out := make([]byte, len(sum))
	copy(out, sum[:])
	return out
}

func tokenPreview(token string) string {
	if len(token) <= 12 {
		return token
	}
	return token[:6] + "..." + token[len(token)-4:]
}
