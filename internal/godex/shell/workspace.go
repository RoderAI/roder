package shell

import (
	"context"
	"fmt"
	"io"
	"strconv"
	"strings"

	"github.com/pandelisz/gode/internal/godex/tools"
	"github.com/pandelisz/gode/internal/godex/workspacepath"
	"mvdan.cc/sh/v3/interp"
)

func RegisterWorkspaceBuiltins(reg *BuiltinRegistry, root string, toolReg *tools.Registry) error {
	if toolReg == nil {
		return fmt.Errorf("tool registry is required")
	}
	root = strings.TrimSpace(root)
	if root == "" {
		root = "."
	}
	builtins := []Builtin{
		{
			Name:        "gode_read_file",
			Description: "read a text file through gode's read_file tool",
			ReadOnly:    true,
			Run: func(ctx context.Context, args []string, stdin io.Reader, stdout io.Writer, stderr io.Writer) error {
				return handleWorkspaceReadFile(ctx, root, toolReg, args, stdout, stderr)
			},
		},
		{
			Name:        "gode_list_files",
			Description: "list workspace directory children through gode's list_files tool",
			ReadOnly:    true,
			Run: func(ctx context.Context, args []string, stdin io.Reader, stdout io.Writer, stderr io.Writer) error {
				return handleWorkspaceListFiles(ctx, root, toolReg, args, stdout, stderr)
			},
		},
		{
			Name:        "gode_search_files",
			Description: "search workspace text files through gode's search_files tool",
			ReadOnly:    true,
			Run: func(ctx context.Context, args []string, stdin io.Reader, stdout io.Writer, stderr io.Writer) error {
				return handleWorkspaceSearchFiles(ctx, toolReg, args, stdout, stderr)
			},
		},
	}
	if toolReg.Has("apply_patch") {
		builtins = append(builtins, Builtin{
			Name:        "gode_apply_patch",
			Description: "apply a patch from stdin through gode's apply_patch tool",
			ReadOnly:    false,
			Run: func(ctx context.Context, args []string, stdin io.Reader, stdout io.Writer, stderr io.Writer) error {
				return handleWorkspaceApplyPatch(ctx, toolReg, stdin, stdout, stderr)
			},
		})
	}
	for _, builtin := range builtins {
		if err := reg.Register(builtin); err != nil {
			return err
		}
	}
	return nil
}

func handleWorkspaceReadFile(ctx context.Context, root string, toolReg *tools.Registry, args []string, stdout io.Writer, stderr io.Writer) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	if len(args) < 2 {
		fmt.Fprintln(stderr, "gode_read_file: path is required")
		return interp.ExitStatus(2)
	}
	if len(args) > 4 {
		fmt.Fprintln(stderr, "gode_read_file: usage: gode_read_file path [start_line] [limit]")
		return interp.ExitStatus(2)
	}
	if _, err := workspacepath.CleanReadPath(root, args[1]); err != nil {
		fmt.Fprintf(stderr, "gode_read_file: %v\n", err)
		return interp.ExitStatus(1)
	}
	input := map[string]any{"path": args[1]}
	if len(args) >= 3 {
		startLine, err := strconv.Atoi(args[2])
		if err != nil {
			fmt.Fprintf(stderr, "gode_read_file: invalid start_line %q\n", args[2])
			return interp.ExitStatus(2)
		}
		input["start_line"] = startLine
	}
	if len(args) >= 4 {
		limit, err := strconv.Atoi(args[3])
		if err != nil {
			fmt.Fprintf(stderr, "gode_read_file: invalid limit %q\n", args[3])
			return interp.ExitStatus(2)
		}
		input["limit"] = limit
	}
	return runWorkspaceTool(ctx, toolReg, tools.Call{Name: "read_file", Input: input}, stdout, stderr)
}

func handleWorkspaceListFiles(ctx context.Context, root string, toolReg *tools.Registry, args []string, stdout io.Writer, stderr io.Writer) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	if len(args) > 2 {
		fmt.Fprintln(stderr, "gode_list_files: usage: gode_list_files [path]")
		return interp.ExitStatus(2)
	}
	path := "."
	if len(args) == 2 {
		path = args[1]
	}
	if _, err := workspacepath.CleanWorkspacePath(root, path); err != nil {
		fmt.Fprintf(stderr, "gode_list_files: %v\n", err)
		return interp.ExitStatus(1)
	}
	return runWorkspaceTool(ctx, toolReg, tools.Call{Name: "list_files", Input: map[string]any{"path": path}}, stdout, stderr)
}

func handleWorkspaceSearchFiles(ctx context.Context, toolReg *tools.Registry, args []string, stdout io.Writer, stderr io.Writer) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	query := strings.TrimSpace(strings.Join(args[1:], " "))
	if query == "" {
		fmt.Fprintln(stderr, "gode_search_files: query is required")
		return interp.ExitStatus(2)
	}
	return runWorkspaceTool(ctx, toolReg, tools.Call{Name: "search_files", Input: map[string]any{"query": query}}, stdout, stderr)
}

func handleWorkspaceApplyPatch(ctx context.Context, toolReg *tools.Registry, stdin io.Reader, stdout io.Writer, stderr io.Writer) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	if stdin == nil {
		fmt.Fprintln(stderr, "gode_apply_patch: patch is required on stdin")
		return interp.ExitStatus(2)
	}
	data, err := io.ReadAll(stdin)
	if err != nil {
		fmt.Fprintf(stderr, "gode_apply_patch: %v\n", err)
		return interp.ExitStatus(1)
	}
	if err := ctx.Err(); err != nil {
		return err
	}
	patch := string(data)
	if strings.TrimSpace(patch) == "" {
		fmt.Fprintln(stderr, "gode_apply_patch: patch is required on stdin")
		return interp.ExitStatus(2)
	}
	return runWorkspaceTool(ctx, toolReg, tools.Call{Name: "apply_patch", Input: map[string]any{"patch": patch}}, stdout, stderr)
}

func runWorkspaceTool(ctx context.Context, toolReg *tools.Registry, call tools.Call, stdout io.Writer, stderr io.Writer) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	result, err := toolReg.Run(ctx, call)
	if err != nil {
		fmt.Fprintln(stderr, err)
		return interp.ExitStatus(1)
	}
	if result.Error != "" {
		writeShellText(stderr, result.Text)
		return interp.ExitStatus(1)
	}
	writeShellText(stdout, result.Text)
	return nil
}

func writeShellText(w io.Writer, text string) {
	if strings.TrimSpace(text) == "" {
		return
	}
	if strings.HasSuffix(text, "\n") {
		fmt.Fprint(w, text)
		return
	}
	fmt.Fprintln(w, text)
}
