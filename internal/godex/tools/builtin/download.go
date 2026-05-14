package builtin

import (
	"context"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"

	"github.com/pandelisz/gode/internal/godex/permission"
	"github.com/pandelisz/gode/internal/godex/tools"
	"github.com/pandelisz/gode/internal/godex/workspacepath"
)

func RegisterDownload(reg *tools.Registry, root string) {
	reg.Register(tools.Tool{
		Name:          "download",
		Description:   "Download a URL into a workspace file.",
		Action:        permission.ActionNetwork,
		Network:       true,
		PathFromInput: pathInput,
		Schema:        objectSchema("url", "path"),
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			path, err := workspacepath.CleanWorkspacePath(root, stringInput(call.Input, "path"))
			if err != nil {
				return tools.Result{}, err
			}
			req, err := http.NewRequestWithContext(ctx, http.MethodGet, stringInput(call.Input, "url"), nil)
			if err != nil {
				return tools.Result{}, err
			}
			resp, err := http.DefaultClient.Do(req)
			if err != nil {
				return tools.Result{}, err
			}
			defer resp.Body.Close()
			if resp.StatusCode < 200 || resp.StatusCode >= 300 {
				return tools.Result{}, fmt.Errorf("download failed: %s", resp.Status)
			}
			if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
				return tools.Result{}, err
			}
			file, err := os.OpenFile(path, os.O_CREATE|os.O_TRUNC|os.O_WRONLY, 0o600)
			if err != nil {
				return tools.Result{}, err
			}
			defer file.Close()
			n, err := io.Copy(file, resp.Body)
			if err != nil {
				return tools.Result{}, err
			}
			return tools.Result{Text: fmt.Sprintf("downloaded %d bytes to %s", n, relPath(root, path))}, nil
		},
	})
}
