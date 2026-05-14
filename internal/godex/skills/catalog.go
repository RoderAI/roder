package skills

import "sort"

func sortedStrings(values []string) []string {
	names := append([]string(nil), values...)
	sort.Strings(names)
	return names
}
