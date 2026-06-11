//! Fuzzy-recovery regression matrix (roadmap phase 80, Task 2): repeated
//! blocks, formatter whitespace drift, line-number echoes, omitted
//! indentation, and ambiguous matches. Fuzzy apply must never fire when two
//! candidates are too close; refusals must carry candidate diagnostics.

use roder_edit_core::{EditApplyError, EditMatchMode, EditOptions, apply_edit};

fn options(fuzzy: EditMatchMode) -> EditOptions {
    EditOptions {
        fuzzy,
        ..EditOptions::default()
    }
}

#[test]
fn repeated_block_refuses_fuzzy_apply() {
    let text = "fn alpha() {\n    work();\n}\n\nfn beta() {\n    work();\n}\n";
    // The needle matches both function bodies after normalization; apply_safe
    // must refuse rather than guess.
    let error = apply_edit(
        "src/lib.rs",
        text,
        "    WORK();",
        "    rest();",
        options(EditMatchMode::ApplySafe),
    )
    .unwrap_err();

    let EditApplyError::OldStringNotFound { candidates, .. } = error else {
        panic!("expected refusal with diagnostics, got {error:?}");
    };
    assert!(
        !candidates.is_empty(),
        "refusal must include candidate snippets"
    );
}

#[test]
fn formatter_whitespace_drift_applies_safely_when_unique() {
    // The file gained trailing whitespace after a formatter pass; the model
    // echoes the old content without it. Normalized matching recovers the
    // unique target in apply_safe mode only.
    let text = "const value = 1;   \nconst other = 2;\n";
    let needle = "const value = 1;\nconst other = 2;";
    let diagnose_error = apply_edit(
        "src/config.ts",
        text,
        needle,
        "const value = 9;\nconst other = 2;",
        options(EditMatchMode::Diagnose),
    )
    .unwrap_err();
    assert!(matches!(
        diagnose_error,
        EditApplyError::OldStringNotFound { .. }
    ));

    let (updated, outcome) = apply_edit(
        "src/config.ts",
        text,
        needle,
        "const value = 9;\nconst other = 2;",
        options(EditMatchMode::ApplySafe),
    )
    .unwrap();
    assert!(updated.contains("const value = 9;"));
    assert_eq!(outcome.replacements, 1);
}

#[test]
fn line_number_echo_is_stripped_before_matching() {
    let text = "alpha\nbeta\ngamma\n";
    let (updated, _) = apply_edit(
        "notes.txt",
        text,
        "    2: beta",
        "BETA",
        EditOptions::default(),
    )
    .unwrap();
    assert_eq!(updated, "alpha\nBETA\ngamma\n");
}

#[test]
fn omitted_indentation_is_restored_when_reindent_is_enabled() {
    let text = "fn main() {\n    let a = 1;\n    let b = 2;\n}\n";
    let (updated, _) = apply_edit(
        "src/main.rs",
        text,
        "    let a = 1;\n    let b = 2;",
        "let a = 1;\nif a > 0 {\n    use_it(a);\n}",
        EditOptions {
            reindent_inserted: true,
            ..EditOptions::default()
        },
    )
    .unwrap();

    assert_eq!(
        updated,
        "fn main() {\n    let a = 1;\n    if a > 0 {\n        use_it(a);\n    }\n}\n",
        "relative indentation inside the inserted block must be preserved"
    );
}

#[test]
fn omitted_indentation_is_left_alone_without_opt_in() {
    let text = "fn main() {\n    let a = 1;\n}\n";
    let (updated, _) = apply_edit(
        "src/main.rs",
        text,
        "    let a = 1;",
        "let a = 2;",
        EditOptions::default(),
    )
    .unwrap();
    assert_eq!(updated, "fn main() {\nlet a = 2;\n}\n");
}

#[test]
fn ambiguous_match_refuses_with_candidate_diagnostics() {
    let text = "import a\nimport a\nimport b\n";
    let error = apply_edit(
        "mod.py",
        text,
        "import a",
        "import c",
        options(EditMatchMode::ApplySafe),
    )
    .unwrap_err();

    let EditApplyError::OldStringAmbiguous {
        occurrences,
        candidates,
        ..
    } = error
    else {
        panic!("expected ambiguity refusal, got {error:?}");
    };
    assert_eq!(occurrences, 2);
    assert!(!candidates.is_empty());
}

#[test]
fn fuzzy_apply_never_fires_below_uniqueness() {
    // Both candidate windows normalize to the same content; even apply_safe
    // refuses because the normalized match is not unique.
    let text = "value()  \nvalue()\n";
    let error = apply_edit(
        "src/x.rs",
        text,
        "VALUE()",
        "other()",
        options(EditMatchMode::ApplySafe),
    )
    .unwrap_err();
    assert!(matches!(error, EditApplyError::OldStringNotFound { .. }));
}
