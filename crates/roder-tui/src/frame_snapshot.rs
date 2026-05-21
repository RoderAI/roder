use ratatui::buffer::Buffer;
use roder_api_transcript::RecordedFrame;
use sha2::{Digest, Sha256};

pub fn normalized_frame_text(buffer: &Buffer) -> String {
    let area = buffer.area;
    let mut rows = Vec::with_capacity(area.height as usize);
    for row in area.y..area.y + area.height {
        let mut line = String::new();
        for column in area.x..area.x + area.width {
            line.push_str(buffer[(column, row)].symbol());
        }
        while line.ends_with(' ') {
            line.pop();
        }
        rows.push(line);
    }
    while rows.last().is_some_and(|row| row.is_empty()) {
        rows.pop();
    }
    rows.join("\n")
}

pub fn frame_text_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity("sha256:".len() + digest.len() * 2);
    out.push_str("sha256:");
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

pub fn recorded_frame(buffer: &Buffer, include_text: bool) -> RecordedFrame {
    let text = normalized_frame_text(buffer);
    RecordedFrame {
        cols: buffer.area.width,
        rows: buffer.area.height,
        text_hash: frame_text_hash(&text),
        text: include_text.then_some(text),
        artifacts: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{
        buffer::Buffer,
        layout::Rect,
        text::Line,
        widgets::{Paragraph, Widget},
    };

    use super::*;

    #[test]
    fn frame_snapshot_normalizes_trailing_blank_space() {
        let area = Rect::new(0, 0, 12, 4);
        let mut buffer = Buffer::empty(area);
        Paragraph::new(vec![Line::raw("hello  "), Line::raw("world")]).render(area, &mut buffer);

        let text = normalized_frame_text(&buffer);

        assert_eq!(text, "hello\nworld");
    }

    #[test]
    fn recorded_frame_can_embed_or_omit_text_with_stable_hash() {
        let area = Rect::new(0, 0, 8, 2);
        let mut buffer = Buffer::empty(area);
        Paragraph::new("abc").render(area, &mut buffer);

        let embedded = recorded_frame(&buffer, true);
        let omitted = recorded_frame(&buffer, false);

        assert_eq!(embedded.text.as_deref(), Some("abc"));
        assert_eq!(omitted.text, None);
        assert_eq!(embedded.text_hash, omitted.text_hash);
        assert!(embedded.text_hash.starts_with("sha256:"));
    }
}
