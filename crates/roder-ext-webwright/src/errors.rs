use std::path::Path;

pub(crate) fn path_display(path: &Path) -> String {
    path.display().to_string()
}
