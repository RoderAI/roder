use crate::types::WebSearchResponse;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderOptions {
    pub max_answer_chars: usize,
    pub max_excerpt_chars: usize,
    pub max_warning_chars: usize,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            max_answer_chars: 1_200,
            max_excerpt_chars: 500,
            max_warning_chars: 240,
        }
    }
}

pub fn render_web_search_response(response: &WebSearchResponse, options: RenderOptions) -> String {
    let mut parts = Vec::new();
    if let Some(answer) = response
        .answer
        .as_deref()
        .filter(|answer| !answer.trim().is_empty())
    {
        parts.push(format!(
            "Answer:\n{}",
            bounded(answer, options.max_answer_chars)
        ));
    }

    if response.results.is_empty() {
        parts.push("No web search results returned.".to_string());
    } else {
        let mut rendered = String::from("Results:");
        for (index, result) in response.results.iter().enumerate() {
            let title = result.title.as_deref().unwrap_or("Untitled result");
            rendered.push_str(&format!("\n{}. {}", index + 1, title));
            if let Some(date) = result
                .published_at
                .as_deref()
                .filter(|date| !date.is_empty())
            {
                rendered.push_str(&format!(" ({date})"));
            }
            rendered.push_str(&format!("\n   {}", result.url));
            if let Some(excerpt) = result
                .snippet
                .as_deref()
                .or(result.content.as_deref())
                .filter(|excerpt| !excerpt.trim().is_empty())
            {
                rendered.push_str(&format!(
                    "\n   {}",
                    bounded(excerpt, options.max_excerpt_chars)
                ));
            }
        }
        parts.push(rendered);
    }

    let warnings: Vec<String> = response
        .warnings
        .iter()
        .filter(|warning| !warning.trim().is_empty())
        .map(|warning| bounded(warning, options.max_warning_chars))
        .collect();
    if !warnings.is_empty() {
        parts.push(format!("Warnings:\n{}", warnings.join("\n")));
    }

    parts.join("\n\n")
}

fn bounded(input: &str, max_chars: usize) -> String {
    let normalized = input.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_chars {
        return normalized;
    }
    let suffix = "...";
    let keep = max_chars.saturating_sub(suffix.len());
    let mut output: String = normalized.chars().take(keep).collect();
    output.push_str(suffix);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_results_with_bounded_text() {
        let mut response = crate::testing::sample_response();
        response.answer = Some("a".repeat(40));
        response.results[0].snippet = Some("b".repeat(40));
        response.warnings = vec!["c".repeat(40)];

        let text = render_web_search_response(
            &response,
            RenderOptions {
                max_answer_chars: 10,
                max_excerpt_chars: 12,
                max_warning_chars: 8,
            },
        );

        assert!(text.contains("Answer:\naaaaaaa..."));
        assert!(text.contains("bbbbbbbbb..."));
        assert!(text.contains("Warnings:\nccccc..."));
        assert!(text.contains("https://example.com/roder"));
    }
}
