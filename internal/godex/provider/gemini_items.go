package provider

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
	"strings"

	"google.golang.org/genai"
)

const geminiSyntheticThoughtSignature = "skip_thought_signature_validator"

type GeminiInput struct {
	SystemInstruction []GeminiPart    `json:"system_instruction,omitempty"`
	Contents          []GeminiContent `json:"contents"`
	Tools             []GeminiTool    `json:"tools,omitempty"`
}

type GeminiContent struct {
	Role  string       `json:"role"`
	Parts []GeminiPart `json:"parts"`
}

type GeminiPart struct {
	Text             string          `json:"text,omitempty"`
	InlineData       *GeminiBlob     `json:"inline_data,omitempty"`
	FunctionCall     *GeminiCall     `json:"function_call,omitempty"`
	FunctionResponse *GeminiResponse `json:"function_response,omitempty"`
	ThoughtSignature []byte          `json:"thought_signature,omitempty"`
}

type GeminiBlob struct {
	MIMEType string `json:"mime_type"`
	Data     []byte `json:"data"`
}

type GeminiCall struct {
	ID   string         `json:"id,omitempty"`
	Name string         `json:"name"`
	Args map[string]any `json:"args,omitempty"`
	Raw  string         `json:"raw,omitempty"`
}

type GeminiResponse struct {
	ID       string         `json:"id,omitempty"`
	Name     string         `json:"name"`
	Response map[string]any `json:"response"`
}

type GeminiTool struct {
	FunctionDeclarations []GeminiFunctionDeclaration `json:"function_declarations"`
}

type GeminiFunctionDeclaration struct {
	Name        string         `json:"name"`
	Description string         `json:"description,omitempty"`
	Parameters  map[string]any `json:"parameters,omitempty"`
}

func GeminiInputFromResponsesItems(items []Item, tools []ToolSpec) (GeminiInput, error) {
	input := GeminiInput{Tools: geminiDebugTools(tools)}
	callNames := map[string]string{}
	for _, item := range items {
		switch item.Kind {
		case ItemMessage:
			parts, err := geminiPartsFromMessage(item)
			if err != nil {
				return GeminiInput{}, err
			}
			if len(parts) == 0 {
				continue
			}
			switch item.Role {
			case string(RoleSystem):
				input.SystemInstruction = append(input.SystemInstruction, parts...)
			case string(RoleAssistant):
				input.appendParts("model", parts)
			default:
				input.appendParts("user", parts)
			}
		case ItemFunctionCall:
			args, raw := geminiToolArgs(item)
			id := firstNonEmpty(item.ToolCallID, item.ID)
			if id != "" && item.ToolName != "" {
				callNames[id] = item.ToolName
			}
			call := GeminiCall{ID: id, Name: item.ToolName, Args: args, Raw: raw}
			signature := geminiThoughtSignature(item.RawJSON)
			if len(signature) == 0 && !input.lastModelContentHasFunctionCall() {
				signature = []byte(geminiSyntheticThoughtSignature)
			}
			input.appendParts("model", []GeminiPart{{FunctionCall: &call, ThoughtSignature: signature}})
		case ItemFunctionOut:
			name := firstNonEmpty(item.ToolName, callNames[item.ToolCallID])
			if name == "" {
				name = "tool"
			}
			input.appendToolResponse(GeminiPart{FunctionResponse: &GeminiResponse{ID: item.ToolCallID, Name: name, Response: geminiToolResponse(item.Text)}})
		case ItemReasoning:
			continue
		case ItemCompaction:
			text := strings.TrimSpace(item.Text)
			if text == "" {
				return GeminiInput{}, NonPortableItemError{ItemID: item.ID, Kind: string(item.Kind), Provider: "gemini", Reason: "provider-specific compaction is nonportable; provider-neutral compaction text is required"}
			}
			input.appendParts("user", []GeminiPart{{Text: text}})
		case ItemRaw:
			if strings.TrimSpace(item.Text) != "" {
				return GeminiInput{}, NonPortableItemError{ItemID: item.ID, Kind: string(item.Kind), Provider: "gemini", Reason: "raw provider-specific text has no Gemini converter"}
			}
			continue
		}
	}
	return input, nil
}

func (input *GeminiInput) appendParts(role string, parts []GeminiPart) {
	if len(parts) == 0 {
		return
	}
	if len(input.Contents) > 0 {
		last := &input.Contents[len(input.Contents)-1]
		if last.Role == role {
			last.Parts = append(last.Parts, parts...)
			return
		}
	}
	input.Contents = append(input.Contents, GeminiContent{Role: role, Parts: parts})
}

func (input *GeminiInput) appendToolResponse(part GeminiPart) {
	if len(input.Contents) > 0 {
		last := &input.Contents[len(input.Contents)-1]
		if last.Role == "user" && geminiUserContentCanAcceptToolResponse(*last) {
			last.Parts = append(last.Parts, part)
			return
		}
	}
	input.Contents = append(input.Contents, GeminiContent{Role: "user", Parts: []GeminiPart{part}})
}

func (input GeminiInput) lastModelContentHasFunctionCall() bool {
	if len(input.Contents) == 0 {
		return false
	}
	last := input.Contents[len(input.Contents)-1]
	if last.Role != "model" {
		return false
	}
	for _, part := range last.Parts {
		if part.FunctionCall != nil {
			return true
		}
	}
	return false
}

func geminiUserContentCanAcceptToolResponse(content GeminiContent) bool {
	for _, part := range content.Parts {
		if part.FunctionResponse == nil {
			return false
		}
	}
	return true
}

func geminiThoughtSignature(raw json.RawMessage) []byte {
	if len(raw) == 0 {
		return nil
	}
	var camel struct {
		ThoughtSignature []byte `json:"thoughtSignature"`
	}
	if json.Unmarshal(raw, &camel) == nil && len(camel.ThoughtSignature) > 0 {
		return append([]byte(nil), camel.ThoughtSignature...)
	}
	var snake struct {
		ThoughtSignature []byte `json:"thought_signature"`
	}
	if json.Unmarshal(raw, &snake) == nil && len(snake.ThoughtSignature) > 0 {
		return append([]byte(nil), snake.ThoughtSignature...)
	}
	return nil
}

func geminiPartsFromMessage(item Item) ([]GeminiPart, error) {
	parts := []GeminiPart{}
	if text := strings.TrimSpace(item.Text); text != "" {
		parts = append(parts, GeminiPart{Text: text})
	}
	for _, image := range item.Images {
		url := strings.TrimSpace(image.URL)
		if url == "" {
			continue
		}
		mediaType, data, ok := parseDataImageURL(url)
		if !ok {
			return nil, NonPortableItemError{ItemID: item.ID, Kind: string(item.Kind), Provider: "gemini", Reason: "image URLs must be data:image/...;base64 URLs for Gemini replay"}
		}
		decoded, err := base64.StdEncoding.DecodeString(data)
		if err != nil {
			return nil, fmt.Errorf("decode Gemini image data for item %s: %w", item.ID, err)
		}
		parts = append(parts, GeminiPart{InlineData: &GeminiBlob{MIMEType: mediaType, Data: decoded}})
	}
	return parts, nil
}

func geminiToolArgs(item Item) (map[string]any, string) {
	raw := chatToolArguments(item)
	args := map[string]any{}
	if strings.TrimSpace(raw) != "" {
		_ = json.Unmarshal([]byte(raw), &args)
	}
	return args, raw
}

func geminiToolResponse(text string) map[string]any {
	text = strings.TrimSpace(text)
	var object map[string]any
	if text != "" && json.Unmarshal([]byte(text), &object) == nil {
		return object
	}
	return map[string]any{"output": text}
}

func geminiDebugTools(tools []ToolSpec) []GeminiTool {
	decls := make([]GeminiFunctionDeclaration, 0, len(tools))
	for _, tool := range tools {
		decls = append(decls, GeminiFunctionDeclaration{Name: strings.TrimSpace(tool.Name), Description: tool.Description, Parameters: geminiToolSchema(tool.Schema)})
	}
	if len(decls) == 0 {
		return nil
	}
	return []GeminiTool{{FunctionDeclarations: decls}}
}

func geminiToolSchema(schema map[string]any) map[string]any {
	if schema == nil {
		return map[string]any{"type": "object", "properties": map[string]any{}}
	}
	return schema
}

func (input GeminiInput) SDKContents() ([]*genai.Content, *genai.Content) {
	contents := make([]*genai.Content, 0, len(input.Contents))
	for _, content := range input.Contents {
		contents = append(contents, content.SDKContent())
	}
	var system *genai.Content
	if len(input.SystemInstruction) > 0 {
		system = (&GeminiContent{Role: "user", Parts: input.SystemInstruction}).SDKContent()
	}
	return contents, system
}

func (content GeminiContent) SDKContent() *genai.Content {
	parts := make([]*genai.Part, 0, len(content.Parts))
	for _, part := range content.Parts {
		parts = append(parts, part.SDKPart())
	}
	role := genai.Role(genai.RoleUser)
	if content.Role == "model" {
		role = genai.RoleModel
	}
	return genai.NewContentFromParts(parts, role)
}

func (part GeminiPart) SDKPart() *genai.Part {
	switch {
	case part.InlineData != nil:
		return genai.NewPartFromBytes(part.InlineData.Data, part.InlineData.MIMEType)
	case part.FunctionCall != nil:
		return &genai.Part{FunctionCall: &genai.FunctionCall{ID: part.FunctionCall.ID, Name: part.FunctionCall.Name, Args: part.FunctionCall.Args}, ThoughtSignature: append([]byte(nil), part.ThoughtSignature...)}
	case part.FunctionResponse != nil:
		return &genai.Part{FunctionResponse: &genai.FunctionResponse{ID: part.FunctionResponse.ID, Name: part.FunctionResponse.Name, Response: part.FunctionResponse.Response}}
	default:
		return genai.NewPartFromText(part.Text)
	}
}

func geminiSDKTools(tools []ToolSpec) []*genai.Tool {
	if len(tools) == 0 {
		return nil
	}
	decls := make([]*genai.FunctionDeclaration, 0, len(tools))
	for _, tool := range tools {
		name := strings.TrimSpace(tool.Name)
		if name == "" {
			continue
		}
		decls = append(decls, &genai.FunctionDeclaration{
			Name:                 name,
			Description:          tool.Description,
			ParametersJsonSchema: geminiToolSchema(tool.Schema),
		})
	}
	if len(decls) == 0 {
		return nil
	}
	return []*genai.Tool{{FunctionDeclarations: decls}}
}
