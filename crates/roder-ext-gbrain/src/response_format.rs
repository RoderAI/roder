//! Model-facing response verbosity control for the `gbrain_*` tools.

use serde::Deserialize;

const CONCISE_CHARS: usize = 240;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseFormat {
    #[default]
    Concise,
    Detailed,
}

impl ResponseFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Concise => "concise",
            Self::Detailed => "detailed",
        }
    }

    /// Bound a context string for concise model output.
    pub fn bound(self, text: &str) -> String {
        match self {
            Self::Detailed => text.to_string(),
            Self::Concise => {
                if text.chars().count() <= CONCISE_CHARS {
                    text.to_string()
                } else {
                    let keep = CONCISE_CHARS.saturating_sub(3);
                    let mut out = text.chars().take(keep).collect::<String>();
                    out.push_str("...");
                    out
                }
            }
        }
    }
}

pub fn schema() -> serde_json::Value {
    serde_json::json!({
        "type": "string",
        "enum": ["concise", "detailed"],
        "default": "concise",
        "description": "concise bounds the model-facing text; detailed returns the full rendered snapshot."
    })
}
