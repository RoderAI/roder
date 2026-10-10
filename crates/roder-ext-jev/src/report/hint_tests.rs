//! The next-step hints that point at other browser tool families.

use super::next_step;

/// The families are named by tool-name prefix, as core's routing block names
/// them. Jev cannot see which are advertised, so the hint must not say they
/// work.
#[test]
fn blocked_hint_names_the_other_families_without_promising_them() {
    let hint = next_step("blocked").expect("blocked has a next step");
    for prefix in ["chrome_*", "browser_use_*", "webwright.*"] {
        assert!(hint.contains(prefix), "{prefix}: {hint}");
    }
    assert!(hint.contains("if one is available"), "{hint}");
    assert!(!hint.contains("another browser tool."), "{hint}");
}

/// A site that refused automated access stays refused: the hint for that
/// status must not send the caller to another family for the same site.
#[test]
fn access_denied_hint_does_not_route_to_other_families() {
    let hint = next_step("access_denied").expect("access_denied has a next step");
    for prefix in ["chrome_", "browser_use_", "webwright"] {
        assert!(!hint.contains(prefix), "{prefix}: {hint}");
    }
}
