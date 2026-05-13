package provider

type ProviderConfig struct {
	ID                 string `json:"id,omitempty" toml:"id,omitempty"`
	Name               string `json:"name,omitempty" toml:"name,omitempty"`
	Kind               string `json:"kind,omitempty" toml:"kind,omitempty"`
	DefaultModel       string `json:"default_model,omitempty" toml:"default_model,omitempty"`
	BaseURL            string `json:"base_url,omitempty" toml:"base_url,omitempty"`
	EnvKey             string `json:"env_key,omitempty" toml:"env_key,omitempty"`
	Disabled           bool   `json:"disabled,omitempty" toml:"disabled,omitempty"`
	RequiresAuth       bool   `json:"requires_auth,omitempty" toml:"requires_auth,omitempty"`
	SupportsWebSockets bool   `json:"supports_websockets,omitempty" toml:"supports_websockets,omitempty"`
}

type SelectedModel struct {
	ID        string `json:"id,omitempty" toml:"id,omitempty"`
	Provider  string `json:"provider,omitempty" toml:"provider,omitempty"`
	Reasoning string `json:"reasoning,omitempty" toml:"reasoning,omitempty"`
}
