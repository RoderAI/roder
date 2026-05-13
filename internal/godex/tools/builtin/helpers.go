package builtin

func objectSchema(required ...string) map[string]any {
	properties := map[string]any{}
	for _, name := range required {
		properties[name] = map[string]any{"type": "string"}
	}
	return map[string]any{
		"type":       "object",
		"properties": properties,
		"required":   required,
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
