use roder_api::skills::SkillSelector;

pub fn parse_skill_invocations(text: &str) -> Vec<SkillSelector> {
    let mut selectors = Vec::new();
    let mut index = 0;
    while let Some(offset) = text[index..].find('$') {
        let dollar = index + offset;
        let rest = &text[dollar + 1..];
        if let Some(stripped) = rest.strip_prefix('{') {
            if let Some(end) = stripped.find('}') {
                let name = &stripped[..end];
                if is_valid_skill_name(name) {
                    selectors.push(SkillSelector::Name {
                        name: name.to_string(),
                    });
                }
                index = dollar + 2 + end + 1;
                continue;
            }
        } else {
            let end = rest
                .char_indices()
                .take_while(|(_, ch)| is_skill_name_char(*ch))
                .map(|(idx, ch)| idx + ch.len_utf8())
                .last()
                .unwrap_or(0);
            if end > 0 {
                selectors.push(SkillSelector::Name {
                    name: rest[..end].to_string(),
                });
                index = dollar + 1 + end;
                continue;
            }
        }
        index = dollar + 1;
    }
    selectors
}

fn is_valid_skill_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(is_skill_name_char)
}

fn is_skill_name_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invocation_parser_supports_plain_and_braced_skill_names() {
        let selectors = parse_skill_invocations("Use $review and ${commit-safely}; ignore $.");

        assert_eq!(
            selectors,
            vec![
                SkillSelector::Name {
                    name: "review".to_string()
                },
                SkillSelector::Name {
                    name: "commit-safely".to_string()
                }
            ]
        );
    }
}
