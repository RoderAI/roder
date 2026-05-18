#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OffsetRange {
    pub anchor: usize,
    pub cursor: usize,
    pub active: bool,
}

impl OffsetRange {
    pub fn new(anchor: usize, cursor: usize) -> Self {
        Self {
            anchor,
            cursor,
            active: true,
        }
    }

    pub fn normalized(self) -> Option<(usize, usize)> {
        if !self.active || self.anchor == self.cursor {
            return None;
        }
        if self.cursor < self.anchor {
            Some((self.cursor, self.anchor))
        } else {
            Some((self.anchor, self.cursor))
        }
    }
}

pub fn selected_offset_text(value: &str, range: OffsetRange) -> String {
    let Some((start, end)) = range.normalized() else {
        return String::new();
    };
    let len = value.chars().count();
    let start = start.min(len);
    let end = end.saturating_add(1).min(len);
    if start >= end {
        return String::new();
    }
    value.chars().skip(start).take(end - start).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_range_normalizes_forward_reverse_collapsed_and_inactive() {
        assert_eq!(OffsetRange::new(1, 4).normalized(), Some((1, 4)));
        assert_eq!(OffsetRange::new(4, 1).normalized(), Some((1, 4)));
        assert_eq!(OffsetRange::new(4, 4).normalized(), None);
        assert_eq!(
            OffsetRange {
                anchor: 1,
                cursor: 4,
                active: false,
            }
            .normalized(),
            None
        );
    }

    #[test]
    fn selected_offset_text_clips_out_of_bounds_and_uses_chars() {
        assert_eq!(
            selected_offset_text("aé日z", OffsetRange::new(1, 99)),
            "é日z"
        );
        assert_eq!(
            selected_offset_text("abcdef", OffsetRange::new(4, 2)),
            "cde"
        );
    }
}
