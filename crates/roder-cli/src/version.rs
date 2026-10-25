/// Handle version probes before configuration, providers, or a terminal are needed.
pub(crate) fn try_print_version() -> anyhow::Result<bool> {
    let raw_args = std::env::args().skip(1).collect::<Vec<_>>();
    let (args, _) = crate::extract_global_config_dir(&raw_args)?;
    if matches!(args.first().map(String::as_str), Some("--version" | "-V")) {
        println!("roder {}", env!("CARGO_PKG_VERSION"));
        return Ok(true);
    }
    Ok(false)
}
