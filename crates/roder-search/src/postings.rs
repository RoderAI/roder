use std::collections::BTreeSet;

pub(crate) type FileId = usize;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct Trigram([u8; 3]);

impl Trigram {
    fn from_window(window: &[u8]) -> Option<Self> {
        if window.len() == 3 && window.iter().all(u8::is_ascii) {
            Some(Self([window[0], window[1], window[2]]))
        } else {
            None
        }
    }
}

pub(crate) fn trigrams(text: &str, case_sensitive: bool) -> BTreeSet<Trigram> {
    let normalized;
    let text = if case_sensitive {
        text
    } else {
        normalized = text.to_ascii_lowercase();
        &normalized
    };

    text.as_bytes()
        .windows(3)
        .filter_map(Trigram::from_window)
        .collect()
}

pub(crate) fn intersect_postings<'a>(
    mut postings: impl Iterator<Item = &'a BTreeSet<FileId>>,
) -> BTreeSet<FileId> {
    let Some(first) = postings.next() else {
        return BTreeSet::new();
    };
    let mut acc = first.clone();
    for posting in postings {
        acc = acc.intersection(posting).copied().collect();
        if acc.is_empty() {
            break;
        }
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_ascii_trigrams() {
        let grams = trigrams("Alphabet", false);
        assert!(grams.contains(&Trigram(*b"alp")));
        assert!(grams.contains(&Trigram(*b"pha")));
        assert!(grams.contains(&Trigram(*b"bet")));
    }
}
