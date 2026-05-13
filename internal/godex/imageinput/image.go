package imageinput

import (
	"encoding/base64"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"strings"
)

type Image struct {
	URL   string
	MIME  string
	Bytes int
}

func EncodeFile(path string) (Image, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return Image{}, err
	}
	return EncodeBytes(filepath.Base(path), data)
}

func EncodeBytes(name string, data []byte) (Image, error) {
	mime := imageMIME(name, data)
	if !supportedMIME(mime) {
		return Image{}, fmt.Errorf("unsupported image %q", mime)
	}
	encoded := base64.StdEncoding.EncodeToString(data)
	return Image{
		URL:   "data:" + mime + ";base64," + encoded,
		MIME:  mime,
		Bytes: len(data),
	}, nil
}

func imageMIME(name string, data []byte) string {
	switch strings.ToLower(filepath.Ext(name)) {
	case ".png":
		return "image/png"
	case ".jpg", ".jpeg":
		return "image/jpeg"
	case ".gif":
		return "image/gif"
	case ".webp":
		return "image/webp"
	}
	return http.DetectContentType(data)
}

func supportedMIME(mime string) bool {
	switch mime {
	case "image/png", "image/jpeg", "image/gif", "image/webp":
		return true
	default:
		return false
	}
}
