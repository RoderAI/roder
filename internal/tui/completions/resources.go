package completions

import (
	"sort"
	"strings"

	"github.com/pandelisz/gode/internal/godex/mcp"
)

type ResourceItem struct {
	Server      string
	URI         string
	Title       string
	Description string
}

func Resources(resources []mcp.Resource, query string, limit int) []ResourceItem {
	query = strings.TrimPrefix(strings.TrimSpace(query), "@")
	serverFilter := ""
	if server, rest, ok := strings.Cut(query, ":"); ok {
		serverFilter = strings.ToLower(server)
		query = rest
	}
	query = strings.ToLower(query)
	if limit <= 0 {
		limit = 50
	}
	var items []ResourceItem
	for _, resource := range resources {
		if serverFilter != "" && strings.ToLower(resource.Server) != serverFilter {
			continue
		}
		haystack := strings.ToLower(resource.URI + " " + resource.Name + " " + resource.Title + " " + resource.Description)
		if query != "" && !strings.Contains(haystack, query) {
			continue
		}
		title := firstNonEmpty(resource.Title, resource.Name, resource.URI)
		items = append(items, ResourceItem{Server: resource.Server, URI: resource.URI, Title: title, Description: resource.Description})
	}
	sort.Slice(items, func(i, j int) bool {
		if items[i].Server == items[j].Server {
			return items[i].URI < items[j].URI
		}
		return items[i].Server < items[j].Server
	})
	if len(items) > limit {
		return items[:limit]
	}
	return items
}

func firstNonEmpty(values ...string) string {
	for _, value := range values {
		if trimmed := strings.TrimSpace(value); trimmed != "" {
			return trimmed
		}
	}
	return ""
}
