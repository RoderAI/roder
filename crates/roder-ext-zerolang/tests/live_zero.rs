use std::env;
use std::fs;
use std::path::PathBuf;

use roder_ext_zerolang::{
    GraphPatchOperation, ZeroCommandRunner, ZerolangConfig, build_patch_text,
};

#[tokio::test]
#[ignore]
async fn live_zero_checked_graph_edit_smoke() -> anyhow::Result<()> {
    if env::var("RODER_ZERO_LIVE").ok().as_deref() != Some("1") {
        return Ok(());
    }
    let runner = ZeroCommandRunner::new(ZerolangConfig::default());
    let doctor = runner
        .run(&["doctor".to_string(), "--json".to_string()], None, true)
        .await?;
    anyhow::ensure!(doctor.success(), "zero doctor failed: {}", doctor.stderr);

    let dir = tempdir("live-zero-edit")?;
    let source = dir.join("main.0");
    fs::write(
        &source,
        "pub fn main(world: World) -> Void raises {\n    check world.out.write(\"hello from zero\\n\")\n}\n",
    )?;
    let input = "main.0".to_string();
    let dump = runner
        .run(
            &[
                "graph".to_string(),
                "dump".to_string(),
                "--json".to_string(),
                input.clone(),
            ],
            Some(&dir),
            true,
        )
        .await?;
    anyhow::ensure!(dump.success(), "zero graph dump failed: {}", dump.stderr);
    let dump_json = dump.json.as_ref().expect("dump must return json");
    let graph_hash = dump_json
        .get("graphHash")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow::anyhow!("graphHash missing from graph dump"))?;
    let literal = dump_json
        .get("nodes")
        .and_then(|nodes| nodes.as_array())
        .and_then(|nodes| {
            nodes.iter().find(|node| {
                node.get("kind").and_then(|value| value.as_str()) == Some("Literal")
                    && node.get("value").and_then(|value| value.as_str())
                        == Some("hello from zero\n")
            })
        })
        .ok_or_else(|| anyhow::anyhow!("string literal node missing from graph dump"))?;
    let node = literal
        .get("id")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow::anyhow!("literal node id missing"))?;
    let patch_text = build_patch_text(
        graph_hash,
        &[GraphPatchOperation {
            op: "set".to_string(),
            node: Some(node.to_string()),
            field: Some("value".to_string()),
            expect: Some("hello from zero\n".to_string()),
            value: Some("hello from roder\n".to_string()),
            kind: None,
            parent: None,
            edge: None,
            order: None,
            name: None,
            ty: None,
            path: None,
            line: None,
            column: None,
            from: None,
            to: None,
            target: None,
            public: None,
            mutable: None,
            static_: None,
            fallible: None,
            export_c: None,
        }],
    )?;
    let patch = runner
        .run(
            &[
                "graph".to_string(),
                "patch".to_string(),
                "--json".to_string(),
                input.clone(),
                "--patch-text".to_string(),
                patch_text,
            ],
            Some(&dir),
            true,
        )
        .await?;
    anyhow::ensure!(patch.success(), "zero graph patch failed: {}", patch.stderr);
    let roundtrip = runner
        .run(
            &[
                "graph".to_string(),
                "roundtrip".to_string(),
                "--json".to_string(),
                input,
            ],
            Some(&dir),
            true,
        )
        .await?;
    anyhow::ensure!(
        roundtrip.success(),
        "zero graph roundtrip failed: {}",
        roundtrip.stderr
    );
    Ok(())
}

fn tempdir(name: &str) -> anyhow::Result<PathBuf> {
    let path = env::temp_dir().join(format!(
        "roder-{name}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&path)?;
    Ok(path)
}
