package agent

import (
	"crypto/sha256"
	"encoding/hex"
	"path/filepath"
	"strings"
	"unicode"
)

func (r *Runner) promptCacheKey() string {
	workspace := strings.TrimSpace(r.workspace)
	if workspace == "" {
		workspace = "."
	}
	if abs, err := filepath.Abs(workspace); err == nil {
		workspace = abs
	} else {
		workspace = filepath.Clean(workspace)
	}
	provider := cacheKeyPart(r.providerName(), "provider")
	model := cacheKeyPart(firstNonEmpty(r.model, "model"), "model")
	seed := strings.Join([]string{"gode", provider, model, workspace}, "\x00")
	sum := sha256.Sum256([]byte(seed))
	return "gode:" + provider + ":" + model + ":" + hex.EncodeToString(sum[:])[:24]
}

func cacheKeyPart(value string, fallback string) string {
	value = strings.TrimSpace(strings.ToLower(value))
	if value == "" {
		value = fallback
	}
	var out strings.Builder
	lastDash := false
	for _, r := range value {
		ok := unicode.IsLetter(r) || unicode.IsDigit(r)
		if ok {
			out.WriteRune(r)
			lastDash = false
			continue
		}
		if !lastDash {
			out.WriteByte('-')
			lastDash = true
		}
	}
	part := strings.Trim(out.String(), "-")
	if part == "" {
		return fallback
	}
	if len(part) > 24 {
		return strings.Trim(part[:24], "-")
	}
	return part
}
