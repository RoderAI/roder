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
        assert_eq!(decode_non_tty_frame(&[0, b'x']).unwrap(), ExecFrame::Stdin(b"x".to_vec()));
        assert_eq!(decode_non_tty_frame(&[4]).unwrap(), ExecFrame::StdinEof);
    }

    #[test]
    fn malformed_frames_fail_closed() {
        assert!(decode_non_tty_frame(&[]).is_err(), "empty frame must error");
        assert!(
            decode_non_tty_frame(&[9, b'x']).is_err(),
            "unknown stream id must error"
        );
    }

    #[test]
    fn decodes_post_exec_binary_stream_body() {
        let output = decode_non_tty_stream(&[1, b'4', b'\n', 3, 0]);

        assert_eq!(output.stdout, "4\n");
        assert_eq!(output.stderr, "");
        assert_eq!(output.exit_code, Some(0));
    }

    #[test]
    fn decodes_interleaved_stdout_and_stderr_with_nonzero_exit() {
        let output = decode_non_tty_stream(&[
            1, b'o', b'u', b't', b'\n', // stdout
            2, b'w', b'a', b'r', b'n', b'\n', // stderr
            1, b'm', b'o', b'r', b'e', // stdout continues
            3, 17, // exit code 17
        ]);

        assert_eq!(output.stdout, "out\nmore");
        assert_eq!(output.stderr, "warn\n");
        assert_eq!(output.exit_code, Some(17));
    }

    #[test]
    fn truncated_exit_frame_defaults_to_success_marker() {
        // Exit marker with no code byte: the stream ended mid-frame; the
        // decoder records exit 0 instead of dropping the marker.
        let output = decode_non_tty_stream(&[1, b'a', 3]);
        assert_eq!(output.stdout, "a");
        assert_eq!(output.exit_code, Some(0));
    }

    #[test]
    fn unknown_leading_bytes_are_preserved_as_stdout() {
        // Streams that start without a known marker byte (e.g. raw TTY data)
        // must not be silently dropped.
        let output = decode_non_tty_stream(&[b'r', b'a', b'w', 3, 1]);
        assert_eq!(output.stdout, "raw");
        assert_eq!(output.exit_code, Some(1));
    }

    #[test]
    fn stdin_eof_markers_are_ignored_in_output_accumulation() {
        let output = decode_non_tty_stream(&[1, b'x', 4, 2, b'y', 3, 0]);
        assert_eq!(output.stdout, "x");
        assert_eq!(output.stderr, "y");
        assert_eq!(output.exit_code, Some(0));
    }
}
