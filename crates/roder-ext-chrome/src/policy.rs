//! Action classification for browser commands.
//!
//! Roder policy gates browser actions in two layers beyond the per-origin site
//! permissions enforced inside the extension:
//!
//! * **Protected** actions (eval, downloads, uploads, navigation, form submits)
//!   require explicit approval and only run in `control` mode.
//! * **Prohibited** actions (CAPTCHA bypass, raw payment/credential handling) are
//!   refused outright and never reach the browser.

use roder_api::chrome::ChromePermissionMode;

/// How a browser command is treated by Roder policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromeActionClass {
    /// Read-only inspection; allowed in `assist`/`control` when the site permits.
    Inspect,
    /// Ordinary interaction (click/type/scroll); allowed in `control`.
    Interact,
    /// High-risk; requires explicit approval and `control` mode.
    Protected,
    /// Refused outright.
    Prohibited,
}

/// Classify a wire command `kind` (e.g. `"page/click"`).
pub fn classify_action(kind: &str) -> ChromeActionClass {
    match kind {
        // Refused outright.
        "page/captcha" | "page/solveCaptcha" => ChromeActionClass::Prohibited,

        // High-risk, approval + control mode.
        "page/eval" | "page/upload" | "page/download" | "tab/navigate" => {
            ChromeActionClass::Protected
        }

        // Interaction.
        "page/click" | "page/type" | "page/keypress" | "page/scroll" | "page/select"
        | "tab/open" | "tab/close" | "tab/activate" | "tabs/group" => ChromeActionClass::Interact,

        // Everything else (snapshot, screenshot, tabs/list, debug reads, chat,
        // permissions queries, recording) is inspection.
        _ => ChromeActionClass::Inspect,
    }
}

/// Returns `Err(reason)` if `kind` may not run under `mode`. The extension also
/// enforces per-origin site permissions; this is the host-side action gate.
pub fn guard(kind: &str, mode: ChromePermissionMode) -> Result<(), String> {
    match classify_action(kind) {
        ChromeActionClass::Prohibited => Err(format!(
            "browser action {kind:?} is prohibited and cannot be performed"
        )),
        ChromeActionClass::Protected if mode != ChromePermissionMode::Control => Err(format!(
            "browser action {kind:?} is protected and requires control mode plus explicit approval"
        )),
        ChromeActionClass::Interact if mode == ChromePermissionMode::Observe => Err(format!(
            "browser action {kind:?} requires assist or control mode"
        )),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prohibited_actions_are_refused_in_every_mode() {
        for mode in [
            ChromePermissionMode::Observe,
            ChromePermissionMode::Assist,
            ChromePermissionMode::Control,
        ] {
            assert!(guard("page/captcha", mode).is_err());
        }
    }

    #[test]
    fn protected_actions_require_control_mode() {
        assert!(guard("page/eval", ChromePermissionMode::Assist).is_err());
        assert!(guard("page/eval", ChromePermissionMode::Control).is_ok());
    }

    #[test]
    fn inspection_allowed_in_assist() {
        assert!(guard("page/snapshot", ChromePermissionMode::Assist).is_ok());
        assert_eq!(classify_action("tabs/list"), ChromeActionClass::Inspect);
    }
}
