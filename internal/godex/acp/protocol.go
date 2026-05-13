package acp

import (
	"bytes"
	"encoding/json"
	"fmt"
)

const (
	jsonrpcVersion = "2.0"

	errorParse          = -32700
	errorInvalidRequest = -32600
	errorMethodNotFound = -32601
	errorInvalidParams  = -32602
	errorInternal       = -32603
	errorNotFound       = -32002
)

type Message struct {
	JSONRPC string    `json:"jsonrpc"`
	ID      any       `json:"-"`
	Method  string    `json:"method,omitempty"`
	Params  any       `json:"params,omitempty"`
	Result  any       `json:"result,omitempty"`
	Error   *RPCError `json:"error,omitempty"`

	hasID bool
}

type RPCError struct {
	Code    int    `json:"code"`
	Message string `json:"message"`
	Data    any    `json:"data,omitempty"`
}

func (m Message) MarshalJSON() ([]byte, error) {
	out := map[string]any{"jsonrpc": jsonrpcVersion}
	if m.hasID {
		out["id"] = m.ID
	}
	if m.Method != "" {
		out["method"] = m.Method
	}
	if m.Params != nil {
		out["params"] = jsonValue(m.Params)
	}
	if m.Result != nil {
		out["result"] = jsonValue(m.Result)
	}
	if m.Error != nil {
		out["error"] = m.Error
	}
	return json.Marshal(out)
}

type inboundMessage struct {
	ID         any
	HasID      bool
	Method     string
	Params     json.RawMessage
	Result     json.RawMessage
	Error      json.RawMessage
	IsResponse bool
	IsRequest  bool
}

func decodeMessage(data []byte) (inboundMessage, *RPCError) {
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.UseNumber()
	var raw struct {
		JSONRPC string           `json:"jsonrpc"`
		ID      *json.RawMessage `json:"id"`
		Method  string           `json:"method"`
		Params  json.RawMessage  `json:"params"`
		Result  json.RawMessage  `json:"result"`
		Error   json.RawMessage  `json:"error"`
	}
	if err := decoder.Decode(&raw); err != nil {
		return inboundMessage{}, rpcError(errorParse, "Parse error")
	}
	if raw.JSONRPC != jsonrpcVersion {
		return inboundMessage{}, rpcError(errorInvalidRequest, "jsonrpc must be \"2.0\"")
	}

	msg := inboundMessage{
		Method: raw.Method,
		Params: raw.Params,
		Result: raw.Result,
		Error:  raw.Error,
	}
	if raw.ID != nil {
		msg.HasID = true
		var id any
		idDecoder := json.NewDecoder(bytes.NewReader(*raw.ID))
		idDecoder.UseNumber()
		if err := idDecoder.Decode(&id); err != nil {
			return inboundMessage{}, rpcError(errorInvalidRequest, "id must be a string, number, or null")
		}
		msg.ID = normalizeJSONValue(id)
	}

	msg.IsResponse = msg.HasID && msg.Method == "" && (raw.Result != nil || raw.Error != nil)
	msg.IsRequest = msg.HasID && msg.Method != ""
	if !msg.IsResponse && msg.Method == "" {
		return inboundMessage{}, rpcError(errorInvalidRequest, "method is required")
	}
	return msg, nil
}

func decodeParams[T any](raw json.RawMessage) (T, error) {
	var params T
	if len(raw) == 0 {
		return params, nil
	}
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.UseNumber()
	if err := decoder.Decode(&params); err != nil {
		return params, err
	}
	return params, nil
}

func requiredParamFields(raw json.RawMessage, fields ...string) error {
	var object map[string]json.RawMessage
	if len(raw) == 0 {
		return fmt.Errorf("params are required")
	}
	if err := json.Unmarshal(raw, &object); err != nil {
		return err
	}
	for _, field := range fields {
		if _, ok := object[field]; !ok {
			return fmt.Errorf("%s is required", field)
		}
	}
	return nil
}

func responseMessage(id any, result any) Message {
	return Message{JSONRPC: jsonrpcVersion, ID: id, Result: jsonValue(result), hasID: true}
}

func errorMessage(id any, err *RPCError) Message {
	return Message{JSONRPC: jsonrpcVersion, ID: id, Error: err, hasID: true}
}

func requestMessage(id any, method string, params any) Message {
	return Message{JSONRPC: jsonrpcVersion, ID: id, Method: method, Params: params, hasID: true}
}

func notificationMessage(method string, params any) Message {
	return Message{JSONRPC: jsonrpcVersion, Method: method, Params: params}
}

func rpcError(code int, message string) *RPCError {
	return &RPCError{Code: code, Message: message}
}

func jsonValue(value any) any {
	if value == nil {
		return map[string]any{}
	}
	data, err := json.Marshal(value)
	if err != nil {
		return value
	}
	var out any
	if err := json.Unmarshal(data, &out); err != nil {
		return value
	}
	return normalizeJSONValue(out)
}

func normalizeJSONValue(value any) any {
	switch value := value.(type) {
	case json.Number:
		if i, err := value.Int64(); err == nil {
			return float64(i)
		}
		if f, err := value.Float64(); err == nil {
			return f
		}
		return value.String()
	case map[string]any:
		for key, item := range value {
			value[key] = normalizeJSONValue(item)
		}
		return value
	case []any:
		for i, item := range value {
			value[i] = normalizeJSONValue(item)
		}
		return value
	default:
		return value
	}
}

func idKey(id any) string {
	data, err := json.Marshal(id)
	if err != nil {
		return fmt.Sprint(id)
	}
	return string(data)
}
