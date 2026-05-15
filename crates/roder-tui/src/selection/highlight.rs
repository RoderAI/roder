use crate::selection::range::{Point, Range, slice_chars};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightSpan {
    pub line: usize,
    pub start_col: usize,
    pub end_col: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightSegment {
    pub text: String,
    pub selected: bool,
}

pub fn highlight_spans(lines: &[String], range: Range) -> Vec<HighlightSpan> {
    let Some((start, end)) = range.normalized() else {
        return Vec::new();
    };
    if lines.is_empty() || start.line >= lines.len() {
        return Vec::new();
    }
    let end_line = end.line.min(lines.len() - 1);
    (start.line..=end_line)
        .filter_map(|line| {
            let width = lines[line].chars().count();
            let start_col = if line == start.line { start.col } else { 0 }.min(width);
            let end_col = if line == end_line { end.col } else { width }.min(width);
            (start_col < end_col).then_some(HighlightSpan {
                line,
                start_col,
                end_col,
            })
        })
        .collect()
}

pub fn highlighted_line_segments(
    line: &str,
    line_idx: usize,
    range: Range,
) -> Vec<HighlightSegment> {
    let Some(span) = highlight_spans(&[line.to_string()], translated_range(line_idx, range))
        .into_iter()
        .next()
    else {
        return vec![HighlightSegment {
            text: line.to_string(),
            selected: false,
        }];
    };
    let mut segments = Vec::new();
    if span.start_col > 0 {
        segments.push(HighlightSegment {
            text: slice_chars(line, 0, span.start_col),
            selected: false,
        });
    }
    segments.push(HighlightSegment {
        text: slice_chars(line, span.start_col, span.end_col),
        selected: true,
    });
    let width = line.chars().count();
    if span.end_col < width {
        segments.push(HighlightSegment {
            text: slice_chars(line, span.end_col, width),
            selected: false,
        });
    }
    segments
}

fn translated_range(line_idx: usize, range: Range) -> Range {
    Range {
        anchor: translate_point(line_idx, range.anchor),
        cursor: translate_point(line_idx, range.cursor),
        active: range.active,
    }
}

fn translate_point(line_idx: usize, point: Point) -> Point {
    if point.line < line_idx {
        Point {
            line: 0,
            col: usize::MIN,
        }
    } else if point.line > line_idx {
        Point {
            line: 0,
            col: usize::MAX,
        }
    } else {
        Point {
            line: 0,
            col: point.col,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlight_spans_cover_multiline_selection() {
        let lines = vec!["abcd".to_string(), "efgh".to_string()];
        let range = Range {
            anchor: Point { line: 0, col: 1 },
            cursor: Point { line: 1, col: 2 },
            active: true,
        };

        assert_eq!(
            highlight_spans(&lines, range),
            vec![
                HighlightSpan {
                    line: 0,
                    start_col: 1,
                    end_col: 4,
                },
                HighlightSpan {
                    line: 1,
                    start_col: 0,
                    end_col: 2,
                }
            ]
        );
    }
}
