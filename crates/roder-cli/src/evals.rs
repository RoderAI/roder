use std::path::{Path, PathBuf};

pub async fn run_eval_cli(args: &[String]) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        Some("run") => {
            let Some(path) = args.get(1) else {
                anyhow::bail!("usage: roder eval run FIXTURE_DIR --offline");
            };
            let offline = args.iter().any(|arg| arg == "--offline");
            if !offline {
                anyhow::bail!("roder eval run currently requires --offline");
            }
            let output_dir = std::env::var_os("RODER_EVAL_OUTPUT_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::temp_dir().join("roder-evals"));
            match roder_evals::run_offline_eval_suite(
                Path::new(path),
                roder_evals::OfflineEvalRunnerOptions {
                    offline,
                    output_dir: output_dir.clone(),
                    provider: roder_api::catalog::PROVIDER_MOCK.to_string(),
                    model: "mock".to_string(),
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
                    offline,
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
        }
        _ => anyhow::bail!("usage: roder eval run FIXTURE_DIR --offline"),
    }
    Ok(())
}
