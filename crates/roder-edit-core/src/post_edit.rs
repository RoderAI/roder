//! Deterministic post-edit actions: bounded indentation normalization for
//! inserted/replaced code and host-provided formatter/validator hooks.
//! These actions are tool-scoped; there are no session-level watcher loops.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidatorPolicy {
    Off,
    Warn,
    Block,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostEditDiagnostic {
    pub kind: String,
    pub message: String,
}

/// Host-provided formatter callback: receives `(path, content)` and returns
/// `Ok(Some(formatted))` to replace the content, `Ok(None)` to leave it
/// unchanged, or `Err` for a formatter failure (reported as a diagnostic,
/// never silently swallowed).
pub type FormatterHook<'a> = &'a dyn Fn(&str, &str) -> anyhow::Result<Option<String>>;

/// Host-provided validator callback: receives `(path, content)` and returns
/// structured diagnostics. The attached policy decides whether diagnostics
/// warn or block the edit.
pub struct PostEditValidator<'a> {
    pub name: &'a str,
    pub policy: ValidatorPolicy,
    pub check: &'a dyn Fn(&str, &str) -> Vec<PostEditDiagnostic>,
}

#[derive(Default)]
pub struct PostEditHooks<'a> {
    pub formatter: Option<FormatterHook<'a>>,
    pub validators: Vec<PostEditValidator<'a>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostEditOutcome {
    pub content: String,
    pub formatted: bool,
    pub diagnostics: Vec<PostEditDiagnostic>,
    /// True when a `Block` validator produced diagnostics; the caller must
    /// not persist `content` in that case.
    pub blocked: bool,
}

/**
 * Run the optional formatter then validators over the edited content for one
 * path. Formatter failures become diagnostics and leave content unchanged;
 * validator behavior follows each validator's policy.
 */
pub fn run_post_edit_hooks(
    path: &str,
    content: &str,
    hooks: &PostEditHooks<'_>,
) -> PostEditOutcome {
    let mut diagnostics = Vec::new();
    let mut formatted = false;
    let mut current = content.to_string();
    if let Some(formatter) = hooks.formatter {
        match formatter(path, &current) {
            Ok(Some(next)) => {
                formatted = next != current;
                current = next;
            }
            Ok(None) => {}
            Err(error) => diagnostics.push(PostEditDiagnostic {
                kind: "formatter_failed".to_string(),
                message: format!("formatter failed for {path}: {error}"),
            }),
        }
    }
    let mut blocked = false;
    for validator in &hooks.validators {
        if matches!(validator.policy, ValidatorPolicy::Off) {
            continue;
        }
        let findings = (validator.check)(path, &current);
        if findings.is_empty() {
            continue;
        }
        if matches!(validator.policy, ValidatorPolicy::Block) {
            blocked = true;
        }
        diagnostics.extend(findings.into_iter().map(|finding| PostEditDiagnostic {
            kind: finding.kind,
            message: format!("[{}] {}", validator.name, finding.message),
        }));
    }
    PostEditOutcome {
        content: current,
        formatted,
        diagnostics,
        blocked,
    }
}

/**
 * Bounded indentation normalization for inserted/replaced multiline code.
 *
 * Models frequently emit replacement blocks at column zero even when the
 * replaced text was indented. When the replaced text has a uniform minimum
 * indentation and the replacement has none, shift every non-empty
 * replacement line right by that minimum indent, preserving relative
 * indentation inside the block. Anything else is left untouched, so the
 * transform is deterministic and scoped to the inserted text only.
 */
pub fn normalize_inserted_indentation(old_string: &str, new_string: &str) -> String {
    let Some(base_indent) = uniform_min_indent(old_string) else {
        return new_string.to_string();
    };
    if base_indent.is_empty() {
        return new_string.to_string();
    }
    // Only reindent when the replacement clearly omitted indentation: its
    // minimum indent across non-empty lines must be zero.
    match uniform_min_indent(new_string) {
        Some(indent) if indent.is_empty() => {}
        _ => return new_string.to_string(),
    }
    new_string
        .split('\n')
        .map(|line| {
            if line.trim().is_empty() {
                line.to_string()
            } else {
                format!("{base_indent}{line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Returns the common leading-whitespace prefix length (as a string) across
/// non-empty lines, or `None` when lines mix tabs and spaces inconsistently.
fn uniform_min_indent(text: &str) -> Option<String> {
    let mut min_indent: Option<&str> = None;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let indent_len = line.len() - line.trim_start().len();
        let indent = &line[..indent_len];
        if indent.chars().any(|ch| ch != ' ' && ch != '\t') {
            return None;
        }
        min_indent = Some(match min_indent {
            None => indent,
            Some(current) => {
                let (short, long) = if indent.len() <= current.len() {
                    (indent, current)
                } else {
                    (current, indent)
                };
                // Mixed tab/space prefixes are not comparable; bail out.
                if !long.starts_with(short) {
                    return None;
                }
                short
            }
        });
    }
    min_indent.map(str::to_string)
}
