//! Regenerate the checked method manifest and its schema from the Rust contract.
fn main() -> std::io::Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/app-server");
    for (name, value) in [
        (
            "roder-app-server.v1.json",
            roder_protocol::schema::app_server_manifest_json(),
        ),
        (
            "methods.schema.json",
            roder_protocol::schema::app_server_json_schema(),
        ),
    ] {
        std::fs::write(
            root.join(name),
            format!("{}\n", serde_json::to_string_pretty(&value)?),
        )?;
    }
    Ok(())
}
