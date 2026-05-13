package appserver

import (
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sort"
	"time"
)

func handleFSReadFile(raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[struct {
		Path string `json:"path"`
	}](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	if err := requireAbsolutePath(params.Path); err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	data, err := os.ReadFile(params.Path)
	if err != nil {
		return nil, rpcError(errorInternal, err.Error())
	}
	return map[string]any{"dataBase64": base64.StdEncoding.EncodeToString(data)}, nil
}

func handleFSWriteFile(raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[struct {
		Path       string `json:"path"`
		DataBase64 string `json:"dataBase64"`
	}](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	if err := requireAbsolutePath(params.Path); err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	data, err := base64.StdEncoding.DecodeString(params.DataBase64)
	if err != nil {
		return nil, rpcError(errorInvalidParams, "dataBase64 is not valid base64")
	}
	if err := os.MkdirAll(filepath.Dir(params.Path), 0o755); err != nil {
		return nil, rpcError(errorInternal, err.Error())
	}
	if err := os.WriteFile(params.Path, data, 0o644); err != nil {
		return nil, rpcError(errorInternal, err.Error())
	}
	return map[string]any{}, nil
}

func handleFSCreateDirectory(raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[struct {
		Path      string `json:"path"`
		Recursive *bool  `json:"recursive"`
	}](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	if err := requireAbsolutePath(params.Path); err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	recursive := true
	if params.Recursive != nil {
		recursive = *params.Recursive
	}
	if recursive {
		err = os.MkdirAll(params.Path, 0o755)
	} else {
		err = os.Mkdir(params.Path, 0o755)
	}
	if err != nil {
		return nil, rpcError(errorInternal, err.Error())
	}
	return map[string]any{}, nil
}

func handleFSGetMetadata(raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[struct {
		Path string `json:"path"`
	}](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	if err := requireAbsolutePath(params.Path); err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	info, err := os.Lstat(params.Path)
	if err != nil {
		return nil, rpcError(errorInternal, err.Error())
	}
	return map[string]any{
		"isDirectory":  info.IsDir(),
		"isFile":       info.Mode().IsRegular(),
		"isSymlink":    info.Mode()&os.ModeSymlink != 0,
		"createdAtMs":  int64(0),
		"modifiedAtMs": info.ModTime().UnixMilli(),
	}, nil
}

func handleFSReadDirectory(raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[struct {
		Path string `json:"path"`
	}](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	if err := requireAbsolutePath(params.Path); err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	entries, err := os.ReadDir(params.Path)
	if err != nil {
		return nil, rpcError(errorInternal, err.Error())
	}
	out := make([]map[string]any, 0, len(entries))
	for _, entry := range entries {
		info, err := entry.Info()
		if err != nil {
			return nil, rpcError(errorInternal, err.Error())
		}
		out = append(out, map[string]any{
			"fileName":    entry.Name(),
			"isDirectory": entry.IsDir(),
			"isFile":      info.Mode().IsRegular(),
		})
	}
	sort.Slice(out, func(i, j int) bool {
		return out[i]["fileName"].(string) < out[j]["fileName"].(string)
	})
	return map[string]any{"entries": out}, nil
}

func handleFSRemove(raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[struct {
		Path      string `json:"path"`
		Recursive *bool  `json:"recursive"`
		Force     *bool  `json:"force"`
	}](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	if err := requireAbsolutePath(params.Path); err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	recursive := true
	if params.Recursive != nil {
		recursive = *params.Recursive
	}
	force := true
	if params.Force != nil {
		force = *params.Force
	}
	if recursive {
		err = os.RemoveAll(params.Path)
	} else {
		err = os.Remove(params.Path)
	}
	if err != nil {
		if force && errors.Is(err, os.ErrNotExist) {
			return map[string]any{}, nil
		}
		return nil, rpcError(errorInternal, err.Error())
	}
	return map[string]any{}, nil
}

func handleFSCopy(raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[struct {
		SourcePath      string `json:"sourcePath"`
		DestinationPath string `json:"destinationPath"`
		Recursive       bool   `json:"recursive"`
	}](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	if err := requireAbsolutePath(params.SourcePath); err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	if err := requireAbsolutePath(params.DestinationPath); err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	info, err := os.Stat(params.SourcePath)
	if err != nil {
		return nil, rpcError(errorInternal, err.Error())
	}
	if info.IsDir() && !params.Recursive {
		return nil, rpcError(errorInvalidParams, "recursive is required for directory copies")
	}
	if err := copyPath(params.SourcePath, params.DestinationPath, info); err != nil {
		return nil, rpcError(errorInternal, err.Error())
	}
	return map[string]any{}, nil
}

func requireAbsolutePath(path string) error {
	if path == "" {
		return fmt.Errorf("path is required")
	}
	if !filepath.IsAbs(path) {
		return fmt.Errorf("path must be absolute")
	}
	return nil
}

func copyPath(source, destination string, info os.FileInfo) error {
	if info.IsDir() {
		if err := os.MkdirAll(destination, info.Mode().Perm()); err != nil {
			return err
		}
		entries, err := os.ReadDir(source)
		if err != nil {
			return err
		}
		for _, entry := range entries {
			entryInfo, err := entry.Info()
			if err != nil {
				return err
			}
			if err := copyPath(filepath.Join(source, entry.Name()), filepath.Join(destination, entry.Name()), entryInfo); err != nil {
				return err
			}
		}
		return nil
	}
	if err := os.MkdirAll(filepath.Dir(destination), 0o755); err != nil {
		return err
	}
	in, err := os.Open(source)
	if err != nil {
		return err
	}
	defer in.Close()

	out, err := os.OpenFile(destination, os.O_CREATE|os.O_TRUNC|os.O_WRONLY, info.Mode().Perm())
	if err != nil {
		return err
	}
	defer out.Close()
	if _, err := io.Copy(out, in); err != nil {
		return err
	}
	return os.Chtimes(destination, time.Now(), info.ModTime())
}
