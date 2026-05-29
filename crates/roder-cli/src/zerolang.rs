use roder_ext_zerolang::{
    GraphPatchOperation, ZeroCommandRunner, ZerolangConfig, build_patch_text,
};

pub(crate) async fn run_zerolang_cli(args: &[String]) -> anyhow::Result<()> {
    let cfg = roder_config::load_config()?;
    let runner = ZeroCommandRunner::new(resolve_config(cfg.zerolang.as_ref()));
    match args.first().map(String::as_str) {
        Some("doctor") => {
            let output = runner
                .run(
                    &["doctor".to_string(), "--json".to_string()],
                    std::env::current_dir().ok().as_deref(),
                    true,
                )
                .await?;
            print_command_output(&output)?;
            ensure_success(&output)?;
        }
        Some("check") => {
            let options = parse_check_options(&args[1..], "check")?;
            let mut argv = vec!["check".to_string(), "--json".to_string()];
            push_target_emit(&mut argv, options.target, options.emit);
            argv.push(options.input);
            let output = runner
                .run(&argv, std::env::current_dir().ok().as_deref(), true)
                .await?;
            print_command_output(&output)?;
            ensure_success(&output)?;
        }
        Some("graph-dump") => {
            let options = parse_graph_output_options(&args[1..], "graph-dump")?;
            let mut argv = vec![
                "graph".to_string(),
                "dump".to_string(),
                "--json".to_string(),
            ];
            if let Some(target) = options.target {
                argv.extend(["--target".to_string(), target]);
            }
            if let Some(out) = options.out {
                argv.extend(["--out".to_string(), out]);
            }
            argv.push(options.input);
            let output = runner
                .run(&argv, std::env::current_dir().ok().as_deref(), true)
                .await?;
            print_command_output(&output)?;
            ensure_success(&output)?;
        }
        Some("edit") => {
            let options = parse_edit_options(&args[1..])?;
            let patch_text = build_patch_text(&options.graph_hash, &options.operations)?;
            if options.dry_run {
                print!("{patch_text}");
                return Ok(());
            }
            let mut argv = vec![
                "graph".to_string(),
                "patch".to_string(),
                "--json".to_string(),
            ];
            if let Some(out) = options.out {
                argv.extend(["--out".to_string(), out]);
            }
            argv.push(options.input);
            argv.extend(["--patch-text".to_string(), patch_text]);
            let output = runner
                .run(&argv, std::env::current_dir().ok().as_deref(), true)
                .await?;
            print_command_output(&output)?;
            ensure_success(&output)?;
        }
        _ => anyhow::bail!(
            "usage: roder zerolang <doctor|check [--target TARGET] [--emit exe|obj] INPUT|graph-dump [--target TARGET] [--out PATH] INPUT|edit INPUT --graph-hash HASH --operation-json JSON [--operation-json JSON] [--out PATH] [--dry-run]>"
        ),
    }
    Ok(())
}

#[derive(Debug, Default, PartialEq, Eq)]
struct CheckOptions {
    input: String,
    target: Option<String>,
    emit: Option<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct GraphOutputOptions {
    input: String,
    target: Option<String>,
    out: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct EditOptions {
    input: String,
    graph_hash: String,
    operations: Vec<GraphPatchOperation>,
    out: Option<String>,
    dry_run: bool,
}

fn resolve_config(cfg: Option<&roder_config::ZerolangConfig>) -> ZerolangConfig {
    cfg.map(|cfg| ZerolangConfig {
        binary: cfg.binary.clone(),
        timeout_seconds: cfg.timeout_seconds,
        artifact_dir: cfg.artifact_dir.clone(),
    })
    .unwrap_or_default()
}

fn parse_check_options(args: &[String], command: &str) -> anyhow::Result<CheckOptions> {
    let mut options = CheckOptions::default();
    let mut positional = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--target" => {
                index += 1;
                options.target = Some(required_arg(args, index, "--target")?.to_string());
            }
            "--emit" => {
                index += 1;
                options.emit = Some(required_arg(args, index, "--emit")?.to_string());
            }
            "--" => {
                positional.extend(args[index + 1..].iter().cloned());
                break;
            }
            value if value.starts_with('-') => {
                anyhow::bail!("unknown roder zerolang {command} option {value}");
            }
            value => positional.push(value.to_string()),
        }
        index += 1;
    }
    options.input = one_positional(positional, command)?;
    Ok(options)
}

fn parse_graph_output_options(
    args: &[String],
    command: &str,
) -> anyhow::Result<GraphOutputOptions> {
    let mut options = GraphOutputOptions::default();
    let mut positional = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--target" => {
                index += 1;
                options.target = Some(required_arg(args, index, "--target")?.to_string());
            }
            "--out" => {
                index += 1;
                options.out = Some(required_arg(args, index, "--out")?.to_string());
            }
            "--" => {
                positional.extend(args[index + 1..].iter().cloned());
                break;
            }
            value if value.starts_with('-') => {
                anyhow::bail!("unknown roder zerolang {command} option {value}");
            }
            value => positional.push(value.to_string()),
        }
        index += 1;
    }
    options.input = one_positional(positional, command)?;
    Ok(options)
}

fn parse_edit_options(args: &[String]) -> anyhow::Result<EditOptions> {
    let mut input = None;
    let mut graph_hash = None;
    let mut operations = Vec::new();
    let mut out = None;
    let mut dry_run = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--graph-hash" => {
                index += 1;
                graph_hash = Some(required_arg(args, index, "--graph-hash")?.to_string());
            }
            "--operation-json" | "--op-json" => {
                index += 1;
                operations.push(serde_json::from_str::<GraphPatchOperation>(required_arg(
                    args,
                    index,
                    "--operation-json",
                )?)?);
            }
            "--out" => {
                index += 1;
                out = Some(required_arg(args, index, "--out")?.to_string());
            }
            "--dry-run" => dry_run = true,
            "--" => {
                for value in &args[index + 1..] {
                    set_single_input(&mut input, value)?;
                }
                break;
            }
            value if value.starts_with('-') => {
                anyhow::bail!("unknown roder zerolang edit option {value}");
            }
            value => set_single_input(&mut input, value)?,
        }
        index += 1;
    }
    let input = input.ok_or_else(|| anyhow::anyhow!("roder zerolang edit requires INPUT"))?;
    let graph_hash =
        graph_hash.ok_or_else(|| anyhow::anyhow!("roder zerolang edit requires --graph-hash"))?;
    if operations.is_empty() {
        anyhow::bail!("roder zerolang edit requires at least one --operation-json");
    }
    Ok(EditOptions {
        input,
        graph_hash,
        operations,
        out,
        dry_run,
    })
}

fn push_target_emit(argv: &mut Vec<String>, target: Option<String>, emit: Option<String>) {
    if let Some(target) = target {
        argv.extend(["--target".to_string(), target]);
    }
    if let Some(emit) = emit {
        argv.extend(["--emit".to_string(), emit]);
    }
}

fn print_command_output(output: &roder_ext_zerolang::ZeroCommandOutput) -> anyhow::Result<()> {
    if let Some(json) = &output.json {
        println!("{}", serde_json::to_string_pretty(json)?);
    } else if output.stdout.trim().is_empty() {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print!("{}", output.stdout);
    }
    Ok(())
}

fn ensure_success(output: &roder_ext_zerolang::ZeroCommandOutput) -> anyhow::Result<()> {
    if output.success() {
        Ok(())
    } else {
        anyhow::bail!("zero command failed with status {:?}", output.status)
    }
}

fn one_positional(values: Vec<String>, command: &str) -> anyhow::Result<String> {
    match values.as_slice() {
        [value] if !value.trim().is_empty() => Ok(value.clone()),
        [] => anyhow::bail!("roder zerolang {command} requires INPUT"),
        _ => anyhow::bail!("roder zerolang {command} accepts one INPUT"),
    }
}

fn set_single_input(input: &mut Option<String>, value: &str) -> anyhow::Result<()> {
    if input.is_some() {
        anyhow::bail!("roder zerolang edit accepts one INPUT");
    }
    if value.trim().is_empty() {
        anyhow::bail!("roder zerolang edit INPUT must not be empty");
    }
    *input = Some(value.to_string());
    Ok(())
}

fn required_arg<'a>(args: &'a [String], index: usize, flag: &str) -> anyhow::Result<&'a str> {
    args.get(index)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("{flag} requires a value"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_edit_options_accepts_dry_run_and_json_operations() {
        let args = [
            "main.0",
            "--graph-hash",
            "graph:f76987e99677f1b3",
            "--operation-json",
            r##"{"op":"rename","node":"#ea5ea1ca","expect":"main","value":"start"}"##,
            "--dry-run",
        ]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();

        let options = parse_edit_options(&args).unwrap();

        assert!(options.dry_run);
        assert_eq!(options.input, "main.0");
        assert_eq!(options.operations[0].op, "rename");
    }

    #[test]
    fn dry_run_patch_text_uses_checked_graph_hash() {
        let args = [
            "main.0",
            "--graph-hash",
            "graph:f76987e99677f1b3",
            "--operation-json",
            r##"{"op":"set","node":"#610c78bf","field":"value","expect":"1","value":"2"}"##,
        ]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();

        let options = parse_edit_options(&args).unwrap();
        let patch_text = build_patch_text(&options.graph_hash, &options.operations).unwrap();

        assert!(patch_text.contains("expect graphHash \"graph:f76987e99677f1b3\""));
        assert!(patch_text.contains("set node=\"#610c78bf\" field=\"value\""));
    }
}
