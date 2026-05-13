package builtin

import (
	"math"
	"strconv"
)

func objectSchema(required ...string) map[string]any {
	properties := map[string]any{}
	for _, name := range required {
		properties[name] = map[string]any{"type": "string"}
	}
	requiredList := append([]string{}, required...)
	return map[string]any{
		"type":       "object",
		"properties": properties,
		"required":   requiredList,
	}
}

func stringInput(input map[string]any, key string) string {
	return stringInputDefault(input, key, "")
}

func stringInputDefault(input map[string]any, key, fallback string) string {
	if input == nil {
		return fallback
	}
	value, ok := input[key]
	if !ok || value == nil {
		return fallback
	}
	if s, ok := value.(string); ok {
		return s
	}
	return fallback
}

func intInputDefault(input map[string]any, key string, fallback int) int {
	if input == nil {
		return fallback
	}
	value, ok := input[key]
	if !ok || value == nil {
		return fallback
	}
	switch v := value.(type) {
	case int:
		return v
	case int64:
		if v > math.MaxInt || v < math.MinInt {
			return fallback
		}
		return int(v)
	case float64:
		if v > math.MaxInt || v < math.MinInt {
			return fallback
		}
		return int(v)
	case string:
		parsed, err := strconv.Atoi(v)
		if err != nil {
			return fallback
		}
		return parsed
	default:
		return fallback
	}
}

func arrayInput(input map[string]any, key string) []any {
	if input == nil {
		return nil
	}
	value, ok := input[key]
	if !ok || value == nil {
		return nil
	}
	if items, ok := value.([]any); ok {
		return items
	}
	return nil
}
