package provider

import "strings"

func IsContextLengthExceeded(err error) bool {
	if err == nil {
		return false
	}
	text := strings.ToLower(err.Error())
	return strings.Contains(text, "context_length_exceeded") ||
		strings.Contains(text, "input exceeds the context window") ||
		strings.Contains(text, "exceeds the context window")
}

func ShouldPruneAfterCompactionError(err error) bool {
	if err == nil {
		return false
	}
	text := strings.ToLower(err.Error())
	return IsContextLengthExceeded(err) ||
		strings.Contains(text, "expected destination type") ||
		strings.Contains(text, "content-type ''") ||
		strings.Contains(text, "not 'application/json'")
}
