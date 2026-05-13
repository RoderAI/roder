package selection

import "github.com/atotto/clipboard"

type ClipboardWriter func(string) error

func SystemClipboardWriter(text string) error {
	return clipboard.WriteAll(text)
}
