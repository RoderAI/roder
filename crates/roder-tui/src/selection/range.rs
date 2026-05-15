#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
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
    pub fn new(anchor: Point) -> Self {
        Self {
            anchor,
            cursor: anchor,
            active: true,
        }
    }

    pub fn update(&mut self, cursor: Point) {
        self.cursor = cursor;
    }

    pub fn clear(&mut self) {
        self.active = false;
    }

    pub fn normalized(&self) -> Option<(Point, Point)> {
        if !self.active || self.anchor == self.cursor {
            return None;
        }
        if self.anchor <= self.cursor {
            Some((self.anchor, self.cursor))
        } else {
            Some((self.cursor, self.anchor))
        }
    }

    pub fn char_count(&self, lines: &[String]) -> usize {
        selected_text(lines, *self).chars().count()
    }
}

pub fn selected_text(lines: &[String], range: Range) -> String {
    let Some((start, end)) = range.normalized() else {
        return String::new();
    };
    if lines.is_empty() || start.line >= lines.len() {
        return String::new();
    }
    let end_line = end.line.min(lines.len() - 1);
    let mut out = Vec::new();
    for (line_idx, line) in lines.iter().enumerate().take(end_line + 1).skip(start.line) {
        let start_col = if line_idx == start.line { start.col } else { 0 };
        let end_col = if line_idx == end_line {
            end.col
        } else {
            line.chars().count()
        };
        out.push(slice_chars(line, start_col, end_col));
    }
    out.join("\n")
}

pub fn slice_chars(value: &str, start: usize, end: usize) -> String {
    let start = start.min(value.chars().count());
    let end = end.min(value.chars().count()).max(start);
    value.chars().skip(start).take(end - start).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_text_clips_columns_and_preserves_lines() {
        let lines = vec!["abcd".to_string(), "ef".to_string(), "ghij".to_string()];
        let range = Range {
            anchor: Point { line: 0, col: 2 },
            cursor: Point { line: 2, col: 3 },
            active: true,
        };

        assert_eq!(selected_text(&lines, range), "cd\nef\nghi");
    }

    #[test]
    fn selected_text_normalizes_reversed_ranges() {
        let lines = vec!["abcd".to_string()];
        let range = Range {
            anchor: Point { line: 0, col: 4 },
            cursor: Point { line: 0, col: 1 },
            active: true,
        };

        assert_eq!(selected_text(&lines, range), "bcd");
    }
}
