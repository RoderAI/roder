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
			"turn/interrupt",
			"model/list",
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
	}
}
