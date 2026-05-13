package skills

import (
	"reflect"
	"testing"
)

func TestNPXInstallCommand(t *testing.T) {
	got := NPXInstallCommand("pandelisz/gode@go-development", InstallScopeGlobal, "/workspace", "/data")
	want := []string{"npx", "--yes", "skills", "add", "pandelisz/gode@go-development", "--global"}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("command = %#v", got)
	}
}

func TestNPXInstallCommandProjectScope(t *testing.T) {
	got := NPXInstallCommand("pandelisz/gode@repo-navigation", InstallScopeProject, "/workspace", "/data")
	want := []string{"npx", "--yes", "skills", "add", "pandelisz/gode@repo-navigation", "--project", "--cwd", "/workspace"}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("command = %#v", got)
	}
}
