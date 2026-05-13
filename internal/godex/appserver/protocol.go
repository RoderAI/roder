package appserver

import (
	"bytes"
	"encoding/json"
	"fmt"
)

const (
	errorInvalidRequest = -32600
	errorMethodNotFound = -32601
	errorInvalidParams  = -32602
	errorInternal       = -32603
)

type Message struct {
	ID     any       `json:"id,omitempty"`
	Method string    `json:"method,omitempty"`
	Params any       `json:"params,omitempty"`
	Result any       `json:"result,omitempty"`
	Error  *RPCError `json:"error,omitempty"`
}

type RPCError struct {
	Code    int    `json:"code"`
	Message string `json:"message"`
	Data    any    `json:"data,omitempty"`
}

type inboundMessage struct {
	ID      any
	HasID   bool
	Method  string
	Params  json.RawMessage
	Result  json.RawMessage
	Error   json.RawMessage
	isEmpty bool
}

func decodeMessage(data []byte) (inboundMessage, error) {
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.UseNumber()
	var raw struct {
		ID     *json.RawMessage `json:"id"`
		Method string           `json:"method"`
		Params json.RawMessage  `json:"params"`
		Result json.RawMessage  `json:"result"`
		Error  json.RawMessage  `json:"error"`
	}
	if err := decoder.Decode(&raw); err != nil {
		return inboundMessage{}, err
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
			return inboundMessage{}, fmt.Errorf("invalid id: %w", err)
		}
		msg.ID = id
	}
	msg.isEmpty = !msg.HasID && msg.Method == "" && raw.Params == nil && raw.Result == nil && raw.Error == nil
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
	return out
}

func rpcError(code int, message string) *RPCError {
	return &RPCError{Code: code, Message: message}
}
