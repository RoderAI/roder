package appserver

func protocolCapabilities() map[string]any {
	return map[string]any{
		"methods": []string{
			"initialize",
			"thread/start",
			"thread/list",
			"thread/loaded/list",
			"thread/read",
			"thread/unsubscribe",
			"turn/start",
			"turn/steer",
			"turn/interrupt",
			"model/list",
			"skills/list",
			"skill/read",
			"skill/setEnabled",
			"mcp/state",
			"mcp/resources/list",
			"mcp/resource/read",
			"lsp/state",
			"lsp/diagnostics",
			"permission/respond",
			"fs/readFile",
			"fs/writeFile",
			"fs/createDirectory",
			"fs/getMetadata",
			"fs/readDirectory",
			"fs/remove",
			"fs/copy",
			"command/exec",
			"command/exec/write",
			"command/exec/terminate",
			"command/exec/resize",
		},
		"turnInput": map[string]any{
			"types":                 []string{"text", "image", "local_image", "file", "local_file"},
			"maxLocalFileBytes":     maxLocalFileInputBytes,
			"localFileBinaryPolicy": "metadata",
		},
	}
}
