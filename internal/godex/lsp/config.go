package lsp

type Config struct {
	Disabled    bool              `json:"disabled,omitempty" toml:"disabled,omitempty"`
	Command     string            `json:"command,omitempty" toml:"command,omitempty"`
	Args        []string          `json:"args,omitempty" toml:"args,omitempty"`
	Env         map[string]string `json:"env,omitempty" toml:"env,omitempty"`
	FileTypes   []string          `json:"filetypes,omitempty" toml:"filetypes,omitempty"`
	RootMarkers []string          `json:"root_markers,omitempty" toml:"root_markers,omitempty"`
	Timeout     int               `json:"timeout,omitempty" toml:"timeout,omitempty"`
}
