#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidatorPolicy {
    Off,
    Warn,
    Block,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostEditDiagnostic {
    pub kind: String,
    pub message: String,
}

pub fn normalize_inserted_indentation(input: &str) -> String {
    input.to_string()
}
