use std::path::{Path, PathBuf};

pub async fn run_eval_cli(args: &[String]) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        Some("run") => run_eval_run(&args[1..]).await?,
        Some("list") => {
            let output_dir = eval_output_dir(args);
            let reports = roder_evals::list_eval_reports(&output_dir)?;
            for report in reports {
                println!(
                    "{}\t{}\t{} passed\t{} failed\t{}",
                    report.id,
                    report.suite_id,
                    report.passed,
                    report.failed,
                    report.path.display()
                );
            }
        }
        Some("report") => {
            let output_dir = eval_output_dir(args);
            let report_id = args
                .get(1)
                .filter(|value| !value.starts_with("--"))
                .map(String::as_str)
                .unwrap_or("eval-run");
            let max_bytes = flag_value(args, "--max-bytes")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(64 * 1024);
            let report = roder_evals::read_eval_report(&output_dir, report_id, max_bytes)?;
            print!("{}", report.markdown);
            if report.truncated {
                println!("\n[truncated]");
            }
        }
        _ => anyhow::bail!(
            "usage: roder eval run FIXTURE_DIR --offline [--speed-policy off|on|both] | roder eval list | roder eval report [REPORT_ID]"
        ),
    }
    Ok(())
}

async fn run_eval_run(args: &[String]) -> anyhow::Result<()> {
    let Some(path) = args.first() else {
        anyhow::bail!("usage: roder eval run FIXTURE_DIR --offline [--speed-policy off|on|both]");
    };
    let offline = args.iter().any(|arg| arg == "--offline");
    let provider = flag_value(args, "--provider")
        .map(ToOwned::to_owned)
        .or_else(|| std::env::var("RODER_PROVIDER").ok());
    let model = flag_value(args, "--model")
        .map(ToOwned::to_owned)
        .or_else(|| std::env::var("RODER_MODEL").ok());
    let runtime_profile = flag_value(args, "--profile")
        .map(str::parse)
        .transpose()?
        .unwrap_or_default();
    let speed_policy = flag_value(args, "--speed-policy")
        .map(str::parse)
        .transpose()?
        .unwrap_or_default();
    if !offline {
        if std::env::var("RODER_EVAL_LIVE_PROVIDER").ok().as_deref() != Some("1") {
            anyhow::bail!(
                "live evals require RODER_EVAL_LIVE_PROVIDER=1 plus --provider and --model"
            );
        }
        if provider.as_deref() != Some("mock") || model.as_deref() != Some("mock") {
            anyhow::bail!("live-provider eval transport is only wired for mock in this phase");
        }
    }
    let output_dir = eval_output_dir(args);
    let provider = provider.unwrap_or_else(|| roder_api::catalog::PROVIDER_MOCK.to_string());
    let model = model.unwrap_or_else(|| "mock".to_string());
    match roder_evals::run_offline_eval_suite(
        Path::new(path),
        roder_evals::OfflineEvalRunnerOptions {
            offline: true,
            output_dir: output_dir.clone(),
            provider,
            model,
            runtime_profile,
            speed_policy,
        },
    )
    .await
    {
        Ok(report) => {
            println!(
                "evaluated {} eval fixtures; report={}; markdown={}",
                report.results.len(),
                output_dir.join("eval-run.json").display(),
                output_dir.join("eval-report.md").display()
            );
            return Ok(());
        }
        Err(err) if err.to_string().contains("no canonical eval fixtures") => {}
        Err(err) => return Err(err),
    }
    let report = roder_evals::run_file_backed_context_eval(
        Path::new(path),
        roder_evals::EvalRunOptions {
            offline: true,
            output_dir: output_dir.clone(),
        },
    )?;
    let benchmark_dir = std::env::var_os("RODER_BENCHMARK_OUTPUT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("benchmark").join("file-backed-dynamic-context"));
    roder_evals::write_file_backed_context_benchmark_markdown(&report, &benchmark_dir)?;
    println!(
        "evaluated {} fixtures; report={}; benchmark={}",
        report.results.len(),
        output_dir.join("file-backed-context-report.json").display(),
        benchmark_dir.display()
    );
    Ok(())
}

fn eval_output_dir(args: &[String]) -> PathBuf {
    flag_value(args, "--output-dir")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("RODER_EVAL_OUTPUT_DIR").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("evals").join("reports"))
}

fn flag_value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|window| window[0] == name)
        .map(|window| window[1].as_str())
}
