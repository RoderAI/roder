package dialogs

import (
	"testing"

	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func TestDialogStackPushPop(t *testing.T) {
	var stack Stack
	stack.Push(KindModels)
	stack.Push(KindCommands)

	if top, ok := stack.Top(); !ok || top != KindCommands {
		t.Fatalf("top = %q, %v", top, ok)
	}
	if got := stack.Len(); got != 2 {
		t.Fatalf("len = %d, want 2", got)
	}
	if popped, ok := stack.Pop(); !ok || popped != KindCommands {
		t.Fatalf("pop = %q, %v", popped, ok)
	}
	if popped, ok := stack.Pop(); !ok || popped != KindModels {
		t.Fatalf("pop = %q, %v", popped, ok)
	}
}

func TestSettingsDialogViewModelUsesModelScreen(t *testing.T) {
	settings := NewSettings(godex.Config{Provider: "mock", Model: "gpt-5.5", Reasoning: godex.ReasoningMedium})
	settings.OpenModels()

	vm := settings.ViewModel()
	if vm == nil {
		t.Fatal("expected settings view model")
	}
	if vm.Screen != viewmodel.SettingsScreenModels {
		t.Fatalf("screen = %q", vm.Screen)
	}
	if len(vm.Models) == 0 {
		t.Fatal("expected model rows")
	}
}

func TestListDialogsWrapSelection(t *testing.T) {
	sessions := NewSessions([]SessionItem{{ID: "s1"}, {ID: "s2"}})
	sessions.Move(-1)
	if got := sessions.SelectedItem().ID; got != "s2" {
		t.Fatalf("session selection = %q", got)
	}

	commands := NewCommands([]CommandItem{{ID: "c1"}, {ID: "c2"}})
	commands.Move(3)
	if got := commands.SelectedItem().ID; got != "c2" {
		t.Fatalf("command selection = %q", got)
	}

	permissions := NewPermissions([]PermissionRequest{{ID: "p1"}, {ID: "p2"}})
	permissions.Move(-1)
	if got := permissions.SelectedRequest().ID; got != "p2" {
		t.Fatalf("permission selection = %q", got)
	}
}
