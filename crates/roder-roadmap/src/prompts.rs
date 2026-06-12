use crate::{DiagnosticSeverity, Document, Task, ValidationResult};

/// Delegation contract injected into every orchestrator-facing roadmap prompt.
///
/// The orchestrator coordinates many workers; it never implements tasks in its
/// own thread. Tool names must match the specs in `tools.rs`.
pub const ORCHESTRATOR_RULES: &str = "Orchestrator contract:\n\
- You are the roadmap orchestrator: triage tasks, delegate work, verify evidence, and keep the document current. Do not implement tasks in this thread.\n\
- Fan out: spawn one worker per independent unchecked task with roadmap_thread_spawn, one call per task. Spawning several workers in a single turn is expected.\n\
- Check roadmap_thread_list before spawning so a task does not get duplicate workers unless the user asks for redundancy.\n\
- Parallelize tasks whose owned paths do not overlap; stagger tasks that share files.\n\
- Keep roughly four workers active unless the user raises or lowers the budget.\n\
- Steer or respawn a blocked worker instead of doing its work yourself.\n\
- Mark a task done only through roadmap_set_task_state with non-empty evidence after its run commands and acceptance criteria pass.";

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
        ORCHESTRATOR_RULES.to_string(),
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
