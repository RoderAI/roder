use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

const DEFAULT_BROWSER: &str = "firefox";
const PLAYWRIGHT_PACKAGE: &str = "playwright";
const RUNTIME_DIR: &str = "python/webwright";
const SETUP_FILE: &str = "setup.json";
const SETUP_RECORD_VERSION: u32 = 1;
const SUPPORTED_BROWSERS: [&str; 3] = ["chromium", "firefox", "webkit"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencyCheckMode {
    Required,
    Skipped,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DependencyReport {
    pub python_command: String,
    pub python_available: bool,
    pub playwright_available: bool,
    pub browser: String,
    pub runtime_dir: Option<String>,
    pub setup_record_found: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebwrightSetupOptions {
    pub browser: Option<String>,
    pub python: Option<String>,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebwrightSetupStepReport {
    pub label: String,
    pub command: Vec<String>,
    pub status: String,
    pub stdout_tail: String,
    pub stderr_tail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebwrightSetupReport {
    pub roder_home: String,
    pub runtime_dir: String,
    pub python: String,
    pub browser: String,
    pub dry_run: bool,
    pub installed: bool,
    pub steps: Vec<WebwrightSetupStepReport>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct WebwrightSetupRecord {
    version: u32,
    roder_home: String,
    runtime_dir: String,
    python: String,
    browser: String,
    installed_at: String,
}

pub fn preflight_local_dependencies(
    mode: DependencyCheckMode,
    requested_browser: Option<&str>,
) -> anyhow::Result<DependencyReport> {
    preflight_local_dependencies_in_roder_home(mode, &roder_config::config_dir(), requested_browser)
}

pub(crate) fn preflight_local_dependencies_in_roder_home(
    mode: DependencyCheckMode,
    roder_home: &Path,
    requested_browser: Option<&str>,
) -> anyhow::Result<DependencyReport> {
    let browser = normalize_browser(requested_browser)?;
    if mode == DependencyCheckMode::Skipped {
        return Ok(DependencyReport {
            python_command: "skipped".to_string(),
            python_available: true,
            playwright_available: true,
            browser,
            runtime_dir: None,
            setup_record_found: false,
            message: "dependency check skipped".to_string(),
        });
    }

    let selected = select_python(roder_home).ok_or_else(|| {
        anyhow::anyhow!(
            "Python 3 is required for Webwright. Install Python 3.10+ or run `roder webwright setup --browser {browser}`."
        )
    })?;
    let playwright_available = Command::new(&selected.python)
        .args(["-c", "import playwright; print('ok')"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if !playwright_available {
        anyhow::bail!(
            "Playwright for Python is required for Webwright. Run `roder webwright setup --browser {browser}` or set RODER_WEBWRIGHT_PYTHON to a Python environment with Playwright installed."
        );
    }
    Ok(DependencyReport {
        python_command: selected.python,
        python_available: true,
        playwright_available: true,
        browser,
        runtime_dir: selected.runtime_dir.map(|path| path.display().to_string()),
        setup_record_found: selected.setup_record_found,
        message: if selected.setup_record_found {
            "Managed Webwright runtime is available".to_string()
        } else {
            "Python and Playwright are available".to_string()
        },
    })
}

pub fn setup_webwright_runtime(
    options: WebwrightSetupOptions,
) -> anyhow::Result<WebwrightSetupReport> {
    setup_webwright_runtime_in_roder_home(&roder_config::config_dir(), options)
}

pub(crate) fn setup_webwright_runtime_in_roder_home(
    roder_home: &Path,
    options: WebwrightSetupOptions,
) -> anyhow::Result<WebwrightSetupReport> {
    let browser = normalize_browser(options.browser.as_deref())?;
    let runtime_dir = runtime_dir(roder_home);
    let venv_dir = runtime_dir.join("venv");
    let venv_python = venv_python_path(&venv_dir);
    let base_python = options
        .python
        .filter(|python| !python.trim().is_empty())
        .or_else(python_command)
        .with_context(|| {
            format!(
                "Python 3 is required to set up Webwright. Install Python 3.10+ and rerun `roder webwright setup --browser {browser}`."
            )
        })?;
    let planned = setup_plan(&base_python, &venv_dir, &venv_python, &browser);
    if options.dry_run {
        return Ok(WebwrightSetupReport {
            roder_home: roder_home.display().to_string(),
            runtime_dir: runtime_dir.display().to_string(),
            python: venv_python.display().to_string(),
            browser,
            dry_run: true,
            installed: false,
            steps: planned
                .into_iter()
                .map(|step| step.report("planned", String::new(), String::new()))
                .collect(),
            message: "Webwright setup dry run; no commands executed".to_string(),
        });
    }

    fs::create_dir_all(&runtime_dir)
        .with_context(|| format!("create Webwright runtime dir {}", runtime_dir.display()))?;
    let mut steps = Vec::new();
    for step in planned {
        steps.push(run_setup_step(step)?);
    }
    let record = WebwrightSetupRecord {
        version: SETUP_RECORD_VERSION,
        roder_home: roder_home.display().to_string(),
        runtime_dir: runtime_dir.display().to_string(),
        python: venv_python.display().to_string(),
        browser: browser.clone(),
        installed_at: time::OffsetDateTime::now_utc().unix_timestamp().to_string(),
    };
    write_setup_record(&runtime_dir, &record)?;

    Ok(WebwrightSetupReport {
        roder_home: roder_home.display().to_string(),
        runtime_dir: runtime_dir.display().to_string(),
        python: venv_python.display().to_string(),
        browser,
        dry_run: false,
        installed: true,
        steps,
        message: "Webwright runtime installed".to_string(),
    })
}

fn python_command() -> Option<String> {
    for candidate in ["python3", "python"] {
        let ok = Command::new(candidate)
            .arg("--version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false);
        if ok {
            return Some(candidate.to_string());
        }
    }
    None
}

struct PythonSelection {
    python: String,
    runtime_dir: Option<PathBuf>,
    setup_record_found: bool,
}

fn select_python(roder_home: &Path) -> Option<PythonSelection> {
    if let Ok(python) = std::env::var("RODER_WEBWRIGHT_PYTHON") {
        if !python.trim().is_empty() {
            return Some(PythonSelection {
                python,
                runtime_dir: None,
                setup_record_found: false,
            });
        }
    }
    if let Some(record) = read_setup_record(roder_home) {
        return Some(PythonSelection {
            python: record.python,
            runtime_dir: Some(PathBuf::from(record.runtime_dir)),
            setup_record_found: true,
        });
    }
    python_command().map(|python| PythonSelection {
        python,
        runtime_dir: None,
        setup_record_found: false,
    })
}

fn normalize_browser(browser: Option<&str>) -> anyhow::Result<String> {
    let browser = browser
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(DEFAULT_BROWSER)
        .trim()
        .to_ascii_lowercase();
    if SUPPORTED_BROWSERS.contains(&browser.as_str()) {
        Ok(browser)
    } else {
        bail!(
            "unsupported Webwright browser {browser:?}; expected one of {}",
            SUPPORTED_BROWSERS.join(", ")
        )
    }
}

struct SetupStep {
    label: &'static str,
    command: Vec<String>,
}

impl SetupStep {
    fn report(
        self,
        status: impl Into<String>,
        stdout_tail: String,
        stderr_tail: String,
    ) -> WebwrightSetupStepReport {
        WebwrightSetupStepReport {
            label: self.label.to_string(),
            command: self.command,
            status: status.into(),
            stdout_tail,
            stderr_tail,
        }
    }
}

fn setup_plan(
    base_python: &str,
    venv_dir: &Path,
    venv_python: &Path,
    browser: &str,
) -> Vec<SetupStep> {
    let venv_python = venv_python.display().to_string();
    vec![
        SetupStep {
            label: "create virtual environment",
            command: vec![
                base_python.to_string(),
                "-m".to_string(),
                "venv".to_string(),
                venv_dir.display().to_string(),
            ],
        },
        SetupStep {
            label: "upgrade pip",
            command: vec![
                venv_python.clone(),
                "-m".to_string(),
                "pip".to_string(),
                "install".to_string(),
                "--upgrade".to_string(),
                "pip".to_string(),
            ],
        },
        SetupStep {
            label: "install Playwright package",
            command: vec![
                venv_python.clone(),
                "-m".to_string(),
                "pip".to_string(),
                "install".to_string(),
                PLAYWRIGHT_PACKAGE.to_string(),
            ],
        },
        SetupStep {
            label: "install Playwright browser",
            command: vec![
                venv_python,
                "-m".to_string(),
                "playwright".to_string(),
                "install".to_string(),
                browser.to_string(),
            ],
        },
    ]
}

fn run_setup_step(step: SetupStep) -> anyhow::Result<WebwrightSetupStepReport> {
    let (program, args) = step
        .command
        .split_first()
        .context("empty Webwright setup command")?;
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("run Webwright setup step {}", step.label))?;
    let stdout_tail = tail_lossy(&output.stdout, 4000);
    let stderr_tail = tail_lossy(&output.stderr, 4000);
    if !output.status.success() {
        bail!(
            "Webwright setup step failed: {}: {}",
            step.label,
            stderr_tail
        );
    }
    Ok(step.report("completed", stdout_tail, stderr_tail))
}

fn write_setup_record(runtime_dir: &Path, record: &WebwrightSetupRecord) -> anyhow::Result<()> {
    fs::write(
        runtime_dir.join(SETUP_FILE),
        serde_json::to_string_pretty(record)?,
    )
    .with_context(|| {
        format!(
            "write Webwright setup record {}",
            runtime_dir.join(SETUP_FILE).display()
        )
    })
}

fn read_setup_record(roder_home: &Path) -> Option<WebwrightSetupRecord> {
    let path = runtime_dir(roder_home).join(SETUP_FILE);
    let text = fs::read_to_string(path).ok()?;
    let record: WebwrightSetupRecord = serde_json::from_str(&text).ok()?;
    if record.version == SETUP_RECORD_VERSION
        && !record.roder_home.trim().is_empty()
        && !record.python.trim().is_empty()
        && !record.runtime_dir.trim().is_empty()
    {
        Some(record)
    } else {
        None
    }
}

fn runtime_dir(roder_home: &Path) -> PathBuf {
    roder_home.join(RUNTIME_DIR)
}

fn venv_python_path(venv_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        venv_dir.join("Scripts").join("python.exe")
    } else {
        venv_dir.join("bin").join("python")
    }
}

fn tail_lossy(bytes: &[u8], max_bytes: usize) -> String {
    if bytes.len() <= max_bytes {
        return String::from_utf8_lossy(bytes).to_string();
    }
    String::from_utf8_lossy(&bytes[bytes.len() - max_bytes..]).to_string()
}
