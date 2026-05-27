use std::process::Command;

use roder_ext_webwright::{WebwrightManifest, WebwrightMode, WebwrightWorkspace, verify_workspace};

#[tokio::test]
#[ignore = "requires RODER_WEBWRIGHT_LIVE=1, RODER_WEBWRIGHT_START_URL, Python, Playwright, and browser binaries"]
async fn live_webwright_final_script_smoke() -> anyhow::Result<()> {
    if std::env::var("RODER_WEBWRIGHT_LIVE").as_deref() != Ok("1") {
        eprintln!("skipping live Webwright smoke; set RODER_WEBWRIGHT_LIVE=1");
        return Ok(());
    }
    let start_url = std::env::var("RODER_WEBWRIGHT_START_URL").map_err(|_| {
        anyhow::anyhow!("RODER_WEBWRIGHT_START_URL is required for live Webwright smoke")
    })?;
    let root = std::env::temp_dir().join(format!(
        "roder-webwright-live-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let workspace = WebwrightWorkspace::new(&root);
    let mut manifest = WebwrightManifest::new(
        "live-smoke",
        format!("Open {start_url} and capture the title"),
        WebwrightMode::Run,
        Some(start_url.clone()),
        Some("firefox".to_string()),
        true,
    );
    manifest.latest_run = Some(1);
    workspace.create(&manifest)?;
    workspace.write_manifest(&manifest)?;
    workspace.write_plan("- [x] CP1: Open the requested live start URL.\n- [x] CP2: Capture a viewport screenshot.\n- [x] CP3: Write the final datum.\n")?;

    let script = live_script(&start_url);
    workspace.write_final_script(&script)?;
    let run_dir = workspace.run_dir(1);
    std::fs::create_dir_all(run_dir.join("screenshots"))?;
    std::fs::write(run_dir.join("final_script.py"), script)?;

    let python = std::env::var("RODER_WEBWRIGHT_PYTHON").unwrap_or_else(|_| "python3".to_string());
    let output = Command::new(python)
        .arg("final_script.py")
        .current_dir(&run_dir)
        .output()?;
    anyhow::ensure!(
        output.status.success(),
        "live Webwright script failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let verification = verify_workspace(&root);
    anyhow::ensure!(
        verification.passed,
        "live Webwright verification failed: {:?}",
        verification.checks
    );
    eprintln!("live Webwright smoke workspace: {}", root.display());
    Ok(())
}

fn live_script(start_url: &str) -> String {
    let start_url = serde_json::to_string(start_url).expect("start URL serializes");
    format!(
        r#"from pathlib import Path
from playwright.sync_api import sync_playwright


def main():
    run_dir = Path(__file__).parent
    screenshots = run_dir / "screenshots"
    screenshots.mkdir(exist_ok=True)
    with sync_playwright() as playwright:
        browser = playwright.firefox.launch(headless=True)
        page = browser.new_page(viewport={{"width": 1280, "height": 1800}})
        page.goto({start_url}, wait_until="domcontentloaded", timeout=30000)
        title = page.title() or page.locator("body").inner_text(timeout=3000)[:80]
        page.screenshot(path=str(screenshots / "final_execution_001_page.png"))
        browser.close()
    (run_dir / "final_script_log.txt").write_text(
        "step 1 action: opened live start URL\n"
        "step 2 action: captured viewport screenshot\n"
        f"final datum: {{title}}\n"
    )


if __name__ == "__main__":
    main()
"#
    )
}
