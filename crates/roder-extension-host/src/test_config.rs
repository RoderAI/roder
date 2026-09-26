/// Config-dependent tests run in a child so they cannot change another test's
/// environment or read the user's saved provider credentials.
pub(super) fn run_in_child(test: &str, default_location: bool) -> bool {
    const MARKER: &str = "RODER_CONFIG_TEST_CHILD";
    if std::env::var(MARKER).as_deref() == Ok(test) {
        return false;
    }
    let directory = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", test, "--nocapture"])
        .env(MARKER, test)
        .env_remove("RODER_CONFIG_DIR")
        .env_remove("RODER_DATA_DIR");
    if !default_location {
        command.env("RODER_CONFIG_DIR", directory.path());
    }
    assert!(
        command.status().unwrap().success(),
        "isolated test failed: {test}"
    );
    true
}
