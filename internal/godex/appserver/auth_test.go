package appserver

import (
	"bytes"
	"net/http"
	"testing"
	"time"
)

func TestGenerateRemoteToken(t *testing.T) {
	fakeRand := bytes.NewReader(bytes.Repeat([]byte{0x42}, remoteTokenBytes))
	token, err := GenerateRemoteToken(fakeRand)
	if err != nil {
		t.Fatalf("generate token: %v", err)
	}
	if len(token.Token) < 40 {
		t.Fatalf("token too short: %d", len(token.Token))
	}
	if token.Preview == token.Token {
		t.Fatalf("preview exposed full token")
	}
	if len(token.Hash) != 32 {
		t.Fatalf("hash length = %d", len(token.Hash))
	}
}

func TestRemoteAuthVerifyRequest(t *testing.T) {
	token, err := GenerateRemoteToken(bytes.NewReader(bytes.Repeat([]byte{0x11}, remoteTokenBytes)))
	if err != nil {
		t.Fatalf("generate token: %v", err)
	}
	auth, err := NewRemoteAuth(token.Token, time.Unix(10, 0))
	if err != nil {
		t.Fatalf("new auth: %v", err)
	}
	if bytes.Contains(auth.TokenHash, []byte(token.Token)) {
		t.Fatalf("auth hash contains plaintext token")
	}

	req, _ := http.NewRequest(http.MethodGet, "http://example.test", nil)
	if auth.VerifyRequest(req) {
		t.Fatal("missing token passed auth")
	}
	req.Header.Set("Authorization", "Bearer "+token.Token)
	if !auth.VerifyRequest(req) {
		t.Fatal("authorization bearer token failed auth")
	}
	req.Header.Set("Authorization", "Bearer wrong")
	if auth.VerifyRequest(req) {
		t.Fatal("wrong bearer token passed auth")
	}

	req.Header.Del("Authorization")
	req.Header.Set("Sec-WebSocket-Protocol", remoteSubprotocol+", bearer."+token.Token)
	if !auth.VerifyRequest(req) {
		t.Fatal("subprotocol bearer token failed auth")
	}
}

func TestRemoteAuthDisabledPassThrough(t *testing.T) {
	req, _ := http.NewRequest(http.MethodGet, "http://example.test", nil)
	if !(RemoteAuth{}).VerifyRequest(req) {
		t.Fatal("disabled auth rejected request")
	}
}
