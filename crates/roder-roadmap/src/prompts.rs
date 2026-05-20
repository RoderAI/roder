use crate::{DiagnosticSeverity, Document, Task, ValidationResult};

#[derive(Debug, Clone)]
pub struct RoadmapPromptInput<'a> {
    pub document: &'a Document,
    pub focused_task: Option<&'a Task>,
    pub validation: Option<&'a ValidationResult>,
    pub skill_body: Option<&'a str>,
}

pub fn roadmap_context_prompt(input: RoadmapPromptInput<'_>) -> String {
    let mut sections = vec![
        "You are in Roder roadmapping mode. Treat the roadmap Markdown document as the primary state; thread transcripts are supporting evidence.".to_string(),
        format!("Document: {}", input.document.title),
        format!("Path: {}", input.document.path.display()),
        format!("Goal: {}", input.document.goal),
    ];
    if let Some(task) = input.focused_task {
        sections.push(format!(
            "Focused task: {} [{}]",
            task.heading,
            if task.checked { "done" } else { "open" }
        ));
        if !task.run_blocks.is_empty() {
            sections.push(format!(
                "Focused task run commands:\n{}",
                task.run_blocks.join("\n")
            ));
        }
    }
    if let Some(validation) = input.validation {
        let diagnostics = validation
            .diagnostics
            .iter()
            .map(|diagnostic| {
                let severity = match diagnostic.severity {
                    DiagnosticSeverity::Error => "error",
                    DiagnosticSeverity::Warning => "warning",
                };
                format!(
                    "- {severity}: {}:{} {}",
                    diagnostic.path.display(),
                    diagnostic
                        .line
                        .map(|line| line.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                    diagnostic.message
                )
            })
            .collect::<Vec<_>>();
        sections.push(if diagnostics.is_empty() {
            "Validation: no diagnostics.".to_string()
        } else {
            format!("Validation diagnostics:\n{}", diagnostics.join("\n"))
        });
    }
    if let Some(skill_body) = input.skill_body.filter(|body| !body.trim().is_empty()) {
        sections.push(format!("Roadmap planning skill:\n{}", skill_body.trim()));
    }
    sections.join("\n\n")
}
