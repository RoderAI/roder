//! Post-edit pipeline tests (roadmap phase 80, Task 3): bounded reindent
//! snapshots and host-provided formatter/validator hooks.

use roder_edit_core::{
    PostEditDiagnostic, PostEditHooks, PostEditValidator, ValidatorPolicy,
    normalize_inserted_indentation, run_post_edit_hooks,
};

#[test]
fn reindent_restores_uniform_indentation_and_preserves_relative_levels() {
    let old = "        if ready {\n            go();\n        }";
    let new = "if ready {\n    go();\n} else {\n    wait();\n}";
    assert_eq!(
        normalize_inserted_indentation(old, new),
        "        if ready {\n            go();\n        } else {\n            wait();\n        }"
    );
}

#[test]
fn reindent_is_noop_when_replacement_already_indented() {
    let old = "    let a = 1;";
    let new = "  let a = 2;";
    assert_eq!(normalize_inserted_indentation(old, new), new);
}

#[test]
fn reindent_is_noop_for_unindented_context_or_mixed_tabs() {
    assert_eq!(normalize_inserted_indentation("top()", "next()"), "next()");
    // Mixed tab/space context is not comparable and must not be touched.
    let mixed_old = "\tone()\n    two()";
    assert_eq!(normalize_inserted_indentation(mixed_old, "three()"), "three()");
}

#[test]
fn reindent_skips_blank_lines() {
    let old = "    a();\n    b();";
    let new = "a();\n\nb();";
    assert_eq!(
        normalize_inserted_indentation(old, new),
        "    a();\n\n    b();"
    );
}

#[test]
fn formatter_hook_rewrites_content_and_reports_failures() {
    let formatter = |_path: &str, content: &str| Ok(Some(format!("{}\n", content.trim_end())));
    let hooks = PostEditHooks {
        formatter: Some(&formatter),
        validators: Vec::new(),
    };
    let outcome = run_post_edit_hooks("src/a.ts", "let x = 1;   ", &hooks);
    assert_eq!(outcome.content, "let x = 1;\n");
    assert!(outcome.formatted);
    assert!(!outcome.blocked);

    let failing = |_path: &str, _content: &str| anyhow::bail!("prettier exited 2");
    let hooks = PostEditHooks {
        formatter: Some(&failing),
        validators: Vec::new(),
    };
    let outcome = run_post_edit_hooks("src/a.ts", "let x = 1;", &hooks);
    assert_eq!(outcome.content, "let x = 1;", "failed formatter must not mutate");
    assert!(!outcome.formatted);
    assert_eq!(outcome.diagnostics.len(), 1);
    assert_eq!(outcome.diagnostics[0].kind, "formatter_failed");
    assert!(outcome.diagnostics[0].message.contains("prettier exited 2"));
}

#[test]
fn validators_follow_warn_block_and_off_policies() {
    let find_todo = |_path: &str, content: &str| {
        if content.contains("TODO") {
            vec![PostEditDiagnostic {
                kind: "todo_left_behind".to_string(),
                message: "remove TODO before saving".to_string(),
            }]
        } else {
            Vec::new()
        }
    };

    let warn = PostEditHooks {
        formatter: None,
        validators: vec![PostEditValidator {
            name: "todo-check",
            policy: ValidatorPolicy::Warn,
            check: &find_todo,
        }],
    };
    let outcome = run_post_edit_hooks("src/a.ts", "// TODO fix", &warn);
    assert!(!outcome.blocked);
    assert_eq!(outcome.diagnostics.len(), 1);
    assert!(outcome.diagnostics[0].message.starts_with("[todo-check]"));

    let block = PostEditHooks {
        formatter: None,
        validators: vec![PostEditValidator {
            name: "todo-check",
            policy: ValidatorPolicy::Block,
            check: &find_todo,
        }],
    };
    let outcome = run_post_edit_hooks("src/a.ts", "// TODO fix", &block);
    assert!(outcome.blocked, "block policy must mark the edit blocked");

    let off = PostEditHooks {
        formatter: None,
        validators: vec![PostEditValidator {
            name: "todo-check",
            policy: ValidatorPolicy::Off,
            check: &find_todo,
        }],
    };
    let outcome = run_post_edit_hooks("src/a.ts", "// TODO fix", &off);
    assert!(outcome.diagnostics.is_empty());
    assert!(!outcome.blocked);
}
