use super::*;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

impl CodexBackend {
    pub(super) async fn spawn_session(
        &self,
        cwd: &std::path::Path,
    ) -> anyhow::Result<CodexSession> {
        let mut child = Command::new(&self.program)
            .arg("app-server")
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("start {} app-server", self.program.display()))?;
        let stdin = child.stdin.take().context("Codex stdin unavailable")?;
        let stdout = child.stdout.take().context("Codex stdout unavailable")?;
        let (tx, rx) = mpsc::channel(256);
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => match serde_json::from_str::<Value>(&line) {
                        Ok(value) => {
                            if tx.send(value).await.is_err() {
                                break;
                            }
                        }
                        Err(err) => {
                            let _ = tx.send(json!({"method":"roder/error","params":{"message":format!("invalid Codex JSON: {err}")}})).await;
                        }
                    },
                    Ok(None) => break,
                    Err(err) => {
                        let _ = tx.send(json!({"method":"roder/error","params":{"message":err.to_string()}})).await;
                        break;
                    }
                }
            }
        });
        let mut session = CodexSession {
            _child: child,
            stdin,
            rx,
            queued: VecDeque::new(),
            derived: VecDeque::new(),
            next_id: 1,
            thread_id: String::new(),
            model: String::new(),
            reasoning: String::new(),
            turn_id: None,
            streamed: HashSet::new(),
            streamed_tool_output: HashSet::new(),
            usage_baseline: None,
            approvals: HashMap::new(),
            snapshot: BackendThreadSnapshot::default(),
        };
        session
            .request(
                "initialize",
                json!({"clientInfo":{
                    "name":"roder", "title":"Roder", "version":env!("CARGO_PKG_VERSION")
                }}),
            )
            .await?;
        session
            .write(&json!({"method":"initialized","params":{}}))
            .await?;
        Ok(session)
    }
}
