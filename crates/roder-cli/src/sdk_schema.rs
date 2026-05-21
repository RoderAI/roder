use std::path::PathBuf;

use roder_protocol::schema::{app_server_json_schema, app_server_manifest_json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SchemaFormat {
    Manifest,
    JsonSchema,
}

pub(crate) fn run_app_server_schema_cli(args: &[String]) -> anyhow::Result<()> {
    let options = parse_schema_options(args)?;
    let value = match options.format {
        SchemaFormat::Manifest => app_server_manifest_json(),
        SchemaFormat::JsonSchema => app_server_json_schema(),
    };
    let text = format!("{}\n", serde_json::to_string_pretty(&value)?);
    if let Some(output) = options.output {
        if let Some(parent) = output.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(output, text)?;
    } else {
        print!("{text}");
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SchemaOptions {
    format: SchemaFormat,
    output: Option<PathBuf>,
}

fn parse_schema_options(args: &[String]) -> anyhow::Result<SchemaOptions> {
    let mut format = SchemaFormat::Manifest;
    let mut output = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--format" => {
                let Some(value) = args.get(i + 1) else {
                    anyhow::bail!("--format requires manifest or json-schema");
                };
                format = parse_format(value)?;
                i += 1;
            }
            arg if arg.starts_with("--format=") => {
                format = parse_format(&arg["--format=".len()..])?;
            }
            "--output" => {
                let Some(value) = args.get(i + 1) else {
                    anyhow::bail!("--output requires a path");
                };
                output = Some(PathBuf::from(value));
                i += 1;
            }
            arg if arg.starts_with("--output=") => {
                output = Some(PathBuf::from(&arg["--output=".len()..]));
            }
            "--help" | "-h" => {
                anyhow::bail!(
                    "usage: roder app-server schema --format manifest|json-schema [--output <path>]"
                );
            }
            other => anyhow::bail!("unknown app-server schema argument: {other}"),
        }
        i += 1;
    }

    Ok(SchemaOptions { format, output })
}

fn parse_format(value: &str) -> anyhow::Result<SchemaFormat> {
    match value {
        "manifest" => Ok(SchemaFormat::Manifest),
        "json-schema" => Ok(SchemaFormat::JsonSchema),
        _ => anyhow::bail!("--format requires manifest or json-schema"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdk_schema_cli_parses_manifest_output() {
        let options = parse_schema_options(&[
            "--format".to_string(),
            "manifest".to_string(),
            "--output".to_string(),
            "target/schema.json".to_string(),
        ])
        .unwrap();

        assert_eq!(options.format, SchemaFormat::Manifest);
        assert_eq!(options.output, Some(PathBuf::from("target/schema.json")));
    }

    #[test]
    fn sdk_schema_cli_parses_json_schema_equals_args() {
        let options = parse_schema_options(&[
            "--format=json-schema".to_string(),
            "--output=target/methods.schema.json".to_string(),
        ])
        .unwrap();

        assert_eq!(options.format, SchemaFormat::JsonSchema);
        assert_eq!(
            options.output,
            Some(PathBuf::from("target/methods.schema.json"))
        );
    }

    #[test]
    fn sdk_schema_cli_rejects_unknown_format() {
        let err = parse_schema_options(&["--format".to_string(), "yaml".to_string()])
            .unwrap_err()
            .to_string();
        assert!(err.contains("manifest or json-schema"));
    }
}
