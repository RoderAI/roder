use std::path::Path;

use roder_ext_webwright::verify_workspace;

fn fixture(path: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../evals/fixtures/webwright")
        .join(path)
}

#[test]
fn verification_reports_success_and_failure_labels() {
    let success = verify_workspace(fixture("basic_success"));
    assert!(success.passed);
    assert_eq!(success.predicted_label, "success");

    let failure = verify_workspace(fixture("missing_log"));
    assert!(!failure.passed);
    assert_eq!(failure.predicted_label, "failure");
    assert!(
        failure
            .checks
            .iter()
            .any(|check| check.message.contains("missing final_script_log.txt"))
    );
}
