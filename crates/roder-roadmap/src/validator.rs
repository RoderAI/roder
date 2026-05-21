use std::collections::HashSet;

use crate::parser::path_diagnostic;
use crate::{Diagnostic, DiagnosticSeverity, Document, ValidationResult};

pub fn validate_document(document: &Document) -> ValidationResult {
    let mut diagnostics = Vec::new();
    if let Some(diagnostic) = path_diagnostic(&document.path) {
        diagnostics.push(diagnostic);
    }
    require(
        &mut diagnostics,
        document,
        !document.title.trim().is_empty(),
        None,
        "missing title heading",
    );
    require(
        &mut diagnostics,
        document,
        !document.goal.trim().is_empty(),
        None,
        "missing **Goal:** field",
    );
    require(
        &mut diagnostics,
        document,
        !document.architecture.trim().is_empty(),
        None,
        "missing **Architecture:** field",
    );
    require(
        &mut diagnostics,
        document,
        !document.owned_paths.is_empty(),
        None,
        "missing owned paths",
    );
    require(
        &mut diagnostics,
        document,
        !document.tasks.is_empty(),
        None,
        "missing task checklist items",
    );
    require(
        &mut diagnostics,
        document,
        document
            .tasks
            .iter()
            .any(|task| !task.run_blocks.is_empty()),
        None,
        "missing Run block",
    );
    require(
        &mut diagnostics,
        document,
        !document.acceptance.is_empty(),
        None,
        "missing acceptance checklist",
    );

    let mut seen = HashSet::new();
    for task in &document.tasks {
        if !seen.insert(task.id.clone()) {
            diagnostics.push(Diagnostic {
                path: document.path.clone(),
                line: Some(task.line),
                severity: DiagnosticSeverity::Error,
                message: format!("duplicate task id: {}", task.id),
            });
        }
    }

    ValidationResult {
        document_id: document.id.clone(),
        diagnostics,
    }
}

fn require(
    diagnostics: &mut Vec<Diagnostic>,
    document: &Document,
    ok: bool,
    line: Option<usize>,
    message: &str,
) {
    if !ok {
        diagnostics.push(Diagnostic {
            path: document.path.clone(),
            line,
            severity: DiagnosticSeverity::Error,
            message: message.to_string(),
        });
    }
}
