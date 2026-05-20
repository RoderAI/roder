use std::fs;
use std::path::Path;

#[test]
fn repo_local_roadmap_planning_skill_is_discoverable() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repo root");
    let path = repo_root.join(".agents/skills/roadmap-planning/SKILL.md");
    let body = fs::read_to_string(path).expect("roadmap-planning skill");

    assert!(body.contains("name: roadmap-planning"));
    assert!(body.contains("description:"));
    assert!(body.contains("roadmap/{NN}-{feature-slug}.md"));
}
