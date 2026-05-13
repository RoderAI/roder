package attachments

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strconv"
	"time"
)

func PasteImageFromClipboard(ctx context.Context, dataDir string) (Attachment, error) {
	dir := filepath.Join(dataDir, "attachments")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return Attachment{}, err
	}
	path := filepath.Join(dir, "pasted-"+time.Now().Format("20060102-150405.000000")+".png")
	if _, err := exec.LookPath("pngpaste"); err == nil {
		cmd := exec.CommandContext(ctx, "pngpaste", path)
		if output, err := cmd.CombinedOutput(); err != nil {
			return Attachment{}, fmt.Errorf("paste image: %w: %s", err, output)
		}
		return New(path), nil
	}
	if runtime.GOOS == "darwin" {
		if err := pasteImageWithAppleScript(ctx, path); err != nil {
			return Attachment{}, err
		}
		return New(path), nil
	}
	return Attachment{}, fmt.Errorf("image paste requires pngpaste on this platform")
}

func pasteImageWithAppleScript(ctx context.Context, path string) error {
	quoted := strconv.Quote(path)
	script := `set outPath to ` + quoted + `
set outFile to POSIX file outPath
set pngData to the clipboard as «class PNGf»
set fileRef to open for access outFile with write permission
set eof of fileRef to 0
write pngData to fileRef
close access fileRef`
	cmd := exec.CommandContext(ctx, "osascript", "-e", script)
	if output, err := cmd.CombinedOutput(); err != nil {
		return fmt.Errorf("paste image: %w: %s", err, output)
	}
	return nil
}
