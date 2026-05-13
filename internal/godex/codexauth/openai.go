package codexauth

import (
	"net/http"

	"github.com/openai/openai-go/v3/option"
)

func OpenAIOptions(dataDir string) []option.RequestOption {
	manager := Manager{Store: Store{DataDir: dataDir}}
	return []option.RequestOption{
		option.WithBaseURL(CodexBaseURL),
		option.WithAPIKey(OAuthDummyKey),
		option.WithHeader("originator", "gode"),
		option.WithMiddleware(func(req *http.Request, next option.MiddlewareNext) (*http.Response, error) {
			access, accountID, err := manager.AccessToken(req.Context())
			if err != nil {
				return nil, err
			}
			req.Header.Set("Authorization", "Bearer "+access)
			req.Header.Set("originator", "gode")
			if req.Header.Get("User-Agent") == "" {
				req.Header.Set("User-Agent", "gode")
			}
			if accountID != "" {
				req.Header.Set("ChatGPT-Account-Id", accountID)
			}
			return next(req)
		}),
	}
}
