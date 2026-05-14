package appserver

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"

	"github.com/google/uuid"
)

type activeUpload struct {
	ID       string
	Path     string
	TempPath string
	Size     int64
	Offset   int64
	file     *os.File
}

func (s *Server) handleFSUploadStart(raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[struct {
		Path      string `json:"path"`
		SizeBytes int64  `json:"sizeBytes,omitempty"`
	}](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	if err := requireAbsolutePath(params.Path); err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	if params.SizeBytes < 0 {
		return nil, rpcError(errorInvalidParams, "sizeBytes must be non-negative")
	}
	if _, err := os.Stat(params.Path); err == nil {
		return nil, rpcError(errorInvalidParams, "destination already exists")
	} else if !os.IsNotExist(err) {
		return nil, rpcError(errorInternal, err.Error())
	}
	if err := os.MkdirAll(filepath.Dir(params.Path), 0o755); err != nil {
		return nil, rpcError(errorInternal, err.Error())
	}
	id := uuid.NewString()
	tmp := filepath.Join(filepath.Dir(params.Path), fmt.Sprintf(".%s.uploading", id))
	file, err := os.OpenFile(tmp, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o600)
	if err != nil {
		return nil, rpcError(errorInternal, err.Error())
	}
	upload := &activeUpload{ID: id, Path: params.Path, TempPath: tmp, Size: params.SizeBytes, file: file}
	s.mu.Lock()
	s.uploads[id] = upload
	s.mu.Unlock()
	return map[string]any{"uploadId": id, "path": params.Path, "offset": int64(0)}, nil
}

func (s *Server) handleFSUploadChunk(raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[struct {
		UploadID   string `json:"uploadId"`
		Offset     int64  `json:"offset"`
		DataBase64 string `json:"dataBase64"`
	}](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	upload := s.uploadByID(params.UploadID)
	if upload == nil {
		return nil, rpcError(errorInvalidParams, "upload not found")
	}
	if params.Offset != upload.Offset {
		return nil, rpcError(errorInvalidParams, fmt.Sprintf("offset mismatch: got %d want %d", params.Offset, upload.Offset))
	}
	data, err := base64.StdEncoding.DecodeString(params.DataBase64)
	if err != nil {
		return nil, rpcError(errorInvalidParams, "dataBase64 is not valid base64")
	}
	if _, err := upload.file.Write(data); err != nil {
		return nil, rpcError(errorInternal, err.Error())
	}
	upload.Offset += int64(len(data))
	return map[string]any{"uploadId": upload.ID, "offset": upload.Offset}, nil
}

func (s *Server) handleFSUploadFinish(raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[struct {
		UploadID string `json:"uploadId"`
	}](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	upload := s.removeUpload(params.UploadID)
	if upload == nil {
		return nil, rpcError(errorInvalidParams, "upload not found")
	}
	if upload.Size > 0 && upload.Offset != upload.Size {
		_ = upload.file.Close()
		_ = os.Remove(upload.TempPath)
		return nil, rpcError(errorInvalidParams, fmt.Sprintf("size mismatch: got %d want %d", upload.Offset, upload.Size))
	}
	if err := upload.file.Close(); err != nil {
		_ = os.Remove(upload.TempPath)
		return nil, rpcError(errorInternal, err.Error())
	}
	if err := os.Rename(upload.TempPath, upload.Path); err != nil {
		_ = os.Remove(upload.TempPath)
		return nil, rpcError(errorInternal, err.Error())
	}
	return map[string]any{"path": upload.Path, "sizeBytes": upload.Offset}, nil
}

func (s *Server) handleFSUploadCancel(raw json.RawMessage) (any, *RPCError) {
	params, err := decodeParams[struct {
		UploadID string `json:"uploadId"`
	}](raw)
	if err != nil {
		return nil, rpcError(errorInvalidParams, err.Error())
	}
	upload := s.removeUpload(params.UploadID)
	if upload == nil {
		return map[string]any{"status": "notFound"}, nil
	}
	_ = upload.file.Close()
	_ = os.Remove(upload.TempPath)
	return map[string]any{"status": "cancelled"}, nil
}

func (s *Server) uploadByID(id string) *activeUpload {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return s.uploads[id]
}

func (s *Server) removeUpload(id string) *activeUpload {
	s.mu.Lock()
	defer s.mu.Unlock()
	upload := s.uploads[id]
	delete(s.uploads, id)
	return upload
}
