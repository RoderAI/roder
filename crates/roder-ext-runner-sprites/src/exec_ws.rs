#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecFrame {
    Stdin(Vec<u8>),
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    Exit(i32),
    StdinEof,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
}

pub fn decode_non_tty_frame(frame: &[u8]) -> anyhow::Result<ExecFrame> {
    let Some((&stream, payload)) = frame.split_first() else {
        anyhow::bail!("sprites exec frame cannot be empty");
    };
    match stream {
        0 => Ok(ExecFrame::Stdin(payload.to_vec())),
        1 => Ok(ExecFrame::Stdout(payload.to_vec())),
        2 => Ok(ExecFrame::Stderr(payload.to_vec())),
        3 => Ok(ExecFrame::Exit(
            payload.first().copied().unwrap_or_default() as i32,
        )),
        4 => Ok(ExecFrame::StdinEof),
        other => anyhow::bail!("unknown sprites exec stream id {other}"),
    }
}

pub fn decode_non_tty_stream(body: &[u8]) -> ExecOutput {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_code = None;
    let mut index = 0;
    while index < body.len() {
        let stream = body[index];
        index += 1;
        match stream {
            1 | 2 => {
                let start = index;
                while index < body.len() && !matches!(body[index], 1 | 2 | 3 | 4) {
                    index += 1;
                }
                if stream == 1 {
                    stdout.extend_from_slice(&body[start..index]);
                } else {
                    stderr.extend_from_slice(&body[start..index]);
                }
            }
            3 => {
                exit_code = body.get(index).map(|code| *code as i32).or(Some(0));
                index = index.saturating_add(1);
            }
            4 => {}
            _ => {
                let start = index - 1;
                while index < body.len() && !matches!(body[index], 1 | 2 | 3 | 4) {
                    index += 1;
                }
                stdout.extend_from_slice(&body[start..index]);
            }
        }
    }
    ExecOutput {
        stdout: String::from_utf8_lossy(&stdout).to_string(),
        stderr: String::from_utf8_lossy(&stderr).to_string(),
        exit_code,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_stream_prefixed_frames() {
        assert_eq!(
            decode_non_tty_frame(&[1, b'h', b'i']).unwrap(),
            ExecFrame::Stdout(b"hi".to_vec())
        );
        assert_eq!(
            decode_non_tty_frame(&[2, b'e']).unwrap(),
            ExecFrame::Stderr(b"e".to_vec())
        );
        assert_eq!(decode_non_tty_frame(&[3, 7]).unwrap(), ExecFrame::Exit(7));
    }

    #[test]
    fn decodes_post_exec_binary_stream_body() {
        let output = decode_non_tty_stream(&[1, b'4', b'\n', 3, 0]);

        assert_eq!(output.stdout, "4\n");
        assert_eq!(output.stderr, "");
        assert_eq!(output.exit_code, Some(0));
    }
}
