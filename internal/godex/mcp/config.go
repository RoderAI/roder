package mcp

import (
	"encoding/json"
	"fmt"
	"strings"
)

type ServerConfig struct {
	Type          string            `json:"type" toml:"type"`
	Command       string            `json:"command,omitempty" toml:"command,omitempty"`
	Args          []string          `json:"args,omitempty" toml:"args,omitempty"`
	Env           map[string]string `json:"env,omitempty" toml:"env,omitempty"`
	URL           string            `json:"url,omitempty" toml:"url,omitempty"`
	Headers       map[string]string `json:"headers,omitempty" toml:"headers,omitempty"`
	Disabled      bool              `json:"disabled,omitempty" toml:"disabled,omitempty"`
	EnabledTools  []string          `json:"enabled_tools,omitempty" toml:"enabled_tools,omitempty"`
	DisabledTools []string          `json:"disabled_tools,omitempty" toml:"disabled_tools,omitempty"`
	Timeout       int               `json:"timeout,omitempty" toml:"timeout,omitempty"`
}

func ParseConfigMap(raw map[string]any) (map[string]ServerConfig, error) {
	out := make(map[string]ServerConfig, len(raw))
	for name, value := range raw {
		cfg, err := ParseConfig(value)
		if err != nil {
			return nil, fmt.Errorf("mcp.%s: %w", name, err)
		}
		out[name] = cfg
	}
	return out, nil
}

func ParseConfig(raw any) (ServerConfig, error) {
	switch value := raw.(type) {
	case ServerConfig:
		return value.withDefaults(), nil
	case map[string]any:
		var cfg ServerConfig
		data, _ := json.Marshal(value)
		if err := json.Unmarshal(data, &cfg); err != nil {
			return ServerConfig{}, err
		}
		return cfg.withDefaults(), nil
	default:
		var cfg ServerConfig
		data, _ := json.Marshal(value)
		if err := json.Unmarshal(data, &cfg); err != nil {
			return ServerConfig{}, err
		}
		return cfg.withDefaults(), nil
	}
}

func (c ServerConfig) withDefaults() ServerConfig {
	c.Type = strings.TrimSpace(c.Type)
	if c.Type == "" {
		c.Type = "stdio"
	}
	if c.Env == nil {
		c.Env = map[string]string{}
	}
	if c.Headers == nil {
		c.Headers = map[string]string{}
	}
	return c
}
