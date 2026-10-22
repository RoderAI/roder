//! Apply file selection before grep pagination so later matching files survive.
use std::path::Path;

pub(super) fn filter_grep_matches(
    matches: Vec<String>,
    glob: Option<&str>,
) -> anyhow::Result<Vec<String>> {
    let Some(glob) = glob.filter(|glob| !glob.is_empty()) else {
        return Ok(matches);
    };
    let matcher = super::compile_glob(glob)?;
    Ok(matches
        .into_iter()
        .filter(|line| {
            let path = line.match_indices(':').find_map(|(index, _)| {
                let (number, _) = line[index + 1..].split_once(':')?;
                number.parse::<u64>().ok().map(|_| &line[..index])
            });
            path.is_some_and(|path| {
                matcher.is_match(path)
                    || Path::new(path)
                        .file_name()
                        .is_some_and(|name| matcher.is_match(name))
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    #[test]
    fn filters_canonical_paths_including_colons() {
        let matches = vec![
            "src/a.txt:1:one",
            "src/a:b.rs:2:two:three",
            "src/b.rs:3:four",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        assert_eq!(
            super::filter_grep_matches(matches, Some("*.rs")).unwrap(),
            ["src/a:b.rs:2:two:three", "src/b.rs:3:four"]
        );
    }
}
