use crate::selection::range::slice_chars;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OffsetRange {
    pub anchor: usize,
    pub cursor: usize,
    pub active: bool,
}

impl OffsetRange {
    pub fn new(anchor: usize) -> Self {
        Self {
            anchor,
            cursor: anchor,
            active: true,
        }
    }

    pub fn update(&mut self, cursor: usize) {
        self.cursor = cursor;
    }

    pub fn clear(&mut self) {
        self.active = false;
    }

    pub fn normalized(&self) -> Option<(usize, usize)> {
        if !self.active || self.anchor == self.cursor {
            return None;
        }
        Some((self.anchor.min(self.cursor), self.anchor.max(self.cursor)))
    }
}

pub fn selected_offset_text(value: &str, range: OffsetRange) -> String {
    let Some((start, end)) = range.normalized() else {
        return String::new();
    };
    slice_chars(value, start, end)
}

pub fn point_to_offset(lines: &[String], line: usize, col: usize) -> usize {
    let mut offset = 0usize;
    for (idx, value) in lines.iter().enumerate() {
        if idx == line {
            return offset + col.min(value.chars().count());
        }
        offset += value.chars().count();
        offset += 1;
    }
    offset
}

pub fn offset_to_point(lines: &[String], offset: usize) -> (usize, usize) {
    let mut remaining = offset;
    for (idx, value) in lines.iter().enumerate() {
        let width = value.chars().count();
        if remaining <= width {
            return (idx, remaining);
        }
        remaining = remaining.saturating_sub(width + 1);
    }
    let last = lines.len().saturating_sub(1);
    let col = lines.last().map(|line| line.chars().count()).unwrap_or(0);
    (last, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_offset_text_uses_char_offsets() {
        let range = OffsetRange {
            anchor: 1,
            cursor: 4,
            active: true,
        };

        assert_eq!(selected_offset_text("aβcd", range), "βcd");
    }

    #[test]
    fn point_offset_round_trip_accounts_for_newlines() {
        let lines = vec!["abc".to_string(), "de".to_string()];

        assert_eq!(point_to_offset(&lines, 1, 1), 5);
        assert_eq!(offset_to_point(&lines, 5), (1, 1));
    }
}
