package appserver

import (
	"fmt"
	"strings"

	qrcode "github.com/skip2/go-qrcode"
)

func RenderTerminalQR(data string) (string, error) {
	data = strings.TrimSpace(data)
	if data == "" {
		return "", fmt.Errorf("qr payload is empty")
	}
	code, err := qrcode.New(data, qrcode.Medium)
	if err != nil {
		return "", fmt.Errorf("render remote qr: %w", err)
	}
	return strings.TrimRight(code.ToSmallString(false), "\n"), nil
}
