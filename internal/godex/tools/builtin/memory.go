package builtin

import (
	"github.com/pandelisz/gode/internal/godex/memory"
	"github.com/pandelisz/gode/internal/godex/tools"
)

func RegisterMemory(reg *tools.Registry, service *memory.Service) {
	memory.RegisterTools(reg, service)
}
