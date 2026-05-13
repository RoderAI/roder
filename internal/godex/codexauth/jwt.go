package codexauth

import (
	"encoding/base64"
	"encoding/json"
	"strings"
)

type Claims struct {
	ChatGPTAccountID string `json:"chatgpt_account_id"`
	Organizations    []struct {
		ID string `json:"id"`
	} `json:"organizations"`
	OpenAIAuth struct {
		ChatGPTAccountID string `json:"chatgpt_account_id"`
	} `json:"https://api.openai.com/auth"`
}

func ExtractAccountID(tokens TokenResponse) string {
	for _, token := range []string{tokens.IDToken, tokens.AccessToken} {
		if claims, ok := parseClaims(token); ok {
			if claims.ChatGPTAccountID != "" {
				return claims.ChatGPTAccountID
			}
			if claims.OpenAIAuth.ChatGPTAccountID != "" {
				return claims.OpenAIAuth.ChatGPTAccountID
			}
			if len(claims.Organizations) > 0 {
				return claims.Organizations[0].ID
			}
		}
	}
	return ""
}

func parseClaims(token string) (Claims, bool) {
	parts := strings.Split(token, ".")
	if len(parts) != 3 {
		return Claims{}, false
	}
	payload, err := base64.RawURLEncoding.DecodeString(parts[1])
	if err != nil {
		return Claims{}, false
	}
	var claims Claims
	if err := json.Unmarshal(payload, &claims); err != nil {
		return Claims{}, false
	}
	return claims, true
}
