pub(crate) fn native_tool_uses_remote_workspace(name: &str) -> bool {
    matches!(
        name,
        "read_file"
            | "list_files"
            | "write_file"
            | "grep"
            | "glob"
            | "edit"
            | "multi_edit"
            | "apply_patch"
            | "shell"
            | "exec_command"
            | "write_stdin"
            | "unified_exec"
            | "view_image"
    ) || name.starts_with("design_")
        || name.starts_with("cua_")
}
