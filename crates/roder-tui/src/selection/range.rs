#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Point {
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Range {
    pub anchor: Point,
    pub cursor: Point,
    pub active: bool,
}

impl Range {
    pub fn new(anchor: Point, cursor: Point) -> Self {
        Self {
            anchor,
            cursor,
            active: true,
        }
    }

    pub fn normalized(self) -> Option<(Point, Point)> {
        if !self.active || self.anchor == self.cursor {
            return None;
        }
        if (self.cursor.line, self.cursor.col) < (self.anchor.line, self.anchor.col) {
            Some((self.cursor, self.anchor))
        } else {
            Some((self.anchor, self.cursor))
        }
    }

    pub fn char_count(self, lines: &[impl AsRef<str>]) -> usize {
        selected_text(lines, self)
            .chars()
            .filter(|ch| *ch != '\n')
            .count()
    }

    pub fn is_copyable(self, lines: &[impl AsRef<str>], min_chars: usize) -> bool {
        self.char_count(lines) >= min_chars
    }
}

pub fn selected_text(lines: &[impl AsRef<str>], range: Range) -> String {
    let Some((start, end)) = range.normalized() else {
        return String::new();
    };
    if start.line >= lines.len() {
        return String::new();
    }

    let end_line = end.line.min(lines.len().saturating_sub(1));
    (start.line..=end_line)
        .filter_map(|line_index| {
            let line = lines[line_index].as_ref();
            let len = line.chars().count();
            let from = if line_index == start.line {
                start.col.min(len)
            } else {
                0
            };
            let to = if line_index == end.line {
                end.col.saturating_add(1).min(len)
            } else {
                len
            };
            (from < to).then(|| slice_chars(line, from, to))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn slice_chars(text: &str, start: usize, end: usize) -> String {
    text.chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_normalizes_forward_reverse_and_inactive_cases() {
        let forward = Range::new(Point { line: 1, col: 2 }, Point { line: 2, col: 3 });
        assert_eq!(
            forward.normalized(),
            Some((Point { line: 1, col: 2 }, Point { line: 2, col: 3 }))
        );

        let reverse = Range::new(Point { line: 4, col: 9 }, Point { line: 2, col: 1 });
        assert_eq!(
            reverse.normalized(),
            Some((Point { line: 2, col: 1 }, Point { line: 4, col: 9 }))
        );

        assert_eq!(
            Range {
                anchor: Point { line: 1, col: 1 },
                cursor: Point { line: 1, col: 1 },
                active: true,
            }
            .normalized(),
            None
        );
        assert_eq!(
            Range {
                anchor: Point { line: 1, col: 1 },
                cursor: Point { line: 1, col: 3 },
                active: false,
            }
            .normalized(),
            None
        );
    }

    #[test]
    fn selected_text_clips_columns_and_preserves_line_breaks() {
        let lines = ["zero", "alpha", "βeta", "omega"];
        let range = Range::new(Point { line: 1, col: 2 }, Point { line: 2, col: 99 });

        assert_eq!(selected_text(&lines, range), "pha\nβeta");
    }

    #[test]
    fn selected_text_uses_character_indexes_not_bytes() {
        let lines = ["aé日z"];
        let range = Range::new(Point { line: 0, col: 1 }, Point { line: 0, col: 2 });

        assert_eq!(selected_text(&lines, range), "é日");
    }

    #[test]
    fn range_char_count_controls_short_copyability() {
        let lines = ["abcdef"];
        assert!(
            !Range::new(Point { line: 0, col: 1 }, Point { line: 0, col: 2 })
                .is_copyable(&lines, 3)
        );
        assert!(
            Range::new(Point { line: 0, col: 1 }, Point { line: 0, col: 3 }).is_copyable(&lines, 3)
        );
    }
}
