//! Tells the model which browser tool family to reach for, when it has more
//! than one (report item 11).
//!
//! Roder ships four ways to drive a browser, and a turn can advertise several
//! of them at once, yet only the description of `browser_use_navigate` says
//! anything about choosing between them. This block adds one line per
//! advertised family.
//!
//! The wording comes from the "Rules of thumb" in
//! `docs/roder-browser-use-provider.md`, which stays the single source of
//! truth. Change both together.
//!
//! The block is a pure function of the advertised tool names (it is built
//! from fixed text, nothing else), so the cached prompt prefix only moves when
//! the tool set does. Core cannot see keys or pairing state, so the block
//! never says a family works; it says any of them can fail at call time.

use roder_api::inference::InstructionBundle;

const HEADING: &str = "## Browser Tool Routing";

/// A way to drive a browser, recognised by tool-name prefix. Declared in the
/// order its line appears in, whatever order the tools arrive in.
#[derive(Clone, Copy)]
enum Family {
    Jev,
    Chrome,
    BrowserUse,
    Webwright,
}

const ORDER: [Family; 4] = [
    Family::Jev,
    Family::Chrome,
    Family::BrowserUse,
    Family::Webwright,
];

impl Family {
    /// The family a tool belongs to. `jev_tab_*` are not one: they only
    /// continue a `jev_browse` run in its own tab.
    fn of(tool: &str) -> Option<Self> {
        if tool == "jev_browse" {
            Some(Self::Jev)
        } else if tool.starts_with("chrome_") {
            Some(Self::Chrome)
        } else if tool.starts_with("browser_use_") {
            Some(Self::BrowserUse)
        } else if tool.starts_with("webwright.") {
            Some(Self::Webwright)
        } else {
            None
        }
    }

    fn bit(self) -> u8 {
        1 << self as u8
    }
}

const INTRO: &str = "More than one browser tool family is on this turn. Pick by task:";

const JEV_LINE: &str = "- `jev_browse`: one bounded goal on a public site or in a fresh \
     session, in a single call. Always pass `success_condition`.";

const CHROME_LINE: &str = "- `chrome_*`: work that needs the user's signed-in Chrome, or \
     debugging.";

/// With Jev on the turn, Chrome is also where a run that Jev could not finish
/// continues. Without it the line must not name a family that is not here.
const CHROME_LINE_AFTER_JEV: &str = "- `chrome_*`: work that needs the user's signed-in Chrome, \
     debugging, or continuing after `jev_browse` stopped at a sign-in wall.";

const BROWSER_USE_LINE: &str =
    "- `browser_use_*`: isolated step-by-step exploration in a fresh browser.";

const WEBWRIGHT_LINE: &str =
    "- `webwright.*`: repeatable or evidence-producing flows, run as scripts.";

/// Core cannot see keys, pairing or installed browsers, so it only says that
/// any family can fail, and what to do then.
const FOOTER: &str = "Any of these can fail at call time (no key, extension not paired, no \
     browser). Then try another family above. Never use one to get around a site's refusal of \
     automated access.";

/// The routing block for the families among `tool_names`, or `None` unless
/// two or more are advertised.
fn block(tool_names: impl IntoIterator<Item = impl AsRef<str>>) -> Option<String> {
    let advertised = tool_names
        .into_iter()
        .filter_map(|name| Family::of(name.as_ref()))
        .fold(0u8, |seen, family| seen | family.bit());
    if advertised.count_ones() < 2 {
        return None;
    }
    let has = |family: Family| advertised & family.bit() != 0;
    let mut lines = vec![HEADING, "", INTRO, ""];
    for family in ORDER.into_iter().filter(|family| has(*family)) {
        lines.push(match family {
            Family::Jev => JEV_LINE,
            Family::Chrome if has(Family::Jev) => CHROME_LINE_AFTER_JEV,
            Family::Chrome => CHROME_LINE,
            Family::BrowserUse => BROWSER_USE_LINE,
            Family::Webwright => WEBWRIGHT_LINE,
        });
    }
    lines.extend(["", FOOTER]);
    Some(lines.join("\n"))
}

/// Append the browser routing block to the developer instructions when two or
/// more browser tool families are on the turn.
pub(crate) fn apply_browser_routing(
    mut instructions: InstructionBundle,
    tool_names: impl IntoIterator<Item = impl AsRef<str>>,
) -> InstructionBundle {
    let Some(block) = block(tool_names) else {
        return instructions;
    };
    // Avoid duplicating the block if a parent already injected it (e.g. resume).
    if instructions
        .developer
        .as_deref()
        .is_some_and(|text| text.contains(HEADING))
    {
        return instructions;
    }
    instructions.developer = Some(match instructions.developer {
        Some(existing) if !existing.trim().is_empty() => format!("{existing}\n\n{block}"),
        _ => block,
    });
    instructions
}

#[cfg(test)]
mod tests {
    use super::*;

    const JEV: &[&str] = &["jev_browse", "jev_tab_look", "jev_tab_click"];
    const CHROME: &[&str] = &["chrome_tabs_list", "chrome_click", "chrome_page_snapshot"];
    const BROWSER_USE: &[&str] = &["browser_use_navigate", "browser_use_get_state"];
    const WEBWRIGHT: &[&str] = &["webwright.run_script", "webwright.verify_run"];
    const OTHER: &[&str] = &["shell", "edit", "web_search", "computer", "view_image"];

    /// (marker the block's line starts with, the family's tools)
    const FAMILIES: [(&str, &[&str]); 4] = [
        ("- `jev_browse`", JEV),
        ("- `chrome_*`", CHROME),
        ("- `browser_use_*`", BROWSER_USE),
        ("- `webwright.*`", WEBWRIGHT),
    ];

    fn names(subset: u8) -> Vec<&'static str> {
        let mut names = OTHER.to_vec();
        for (index, (_, tools)) in FAMILIES.iter().enumerate() {
            if subset & (1 << index) != 0 {
                names.extend_from_slice(tools);
            }
        }
        names
    }

    fn all_four() -> String {
        block(names(0b1111)).expect("block with every family")
    }

    #[test]
    fn absent_with_zero_or_one_family() {
        assert_eq!(block(Vec::<&str>::new()), None);
        assert_eq!(block(OTHER.iter().copied()), None);
        for (index, (marker, _)) in FAMILIES.iter().enumerate() {
            assert_eq!(block(names(1 << index)), None, "one family ({marker})");
        }
        // The Jev hand-over tools continue a jev_browse run; they are not a
        // second family beside the Chrome tools.
        let hand_over_with_chrome: Vec<&str> =
            ["jev_tab_look", "jev_tab_click", "chrome_click"].to_vec();
        assert_eq!(block(hand_over_with_chrome), None);
    }

    #[test]
    fn lists_only_advertised_families() {
        for subset in 0u8..16 {
            let advertised = subset.count_ones() as usize;
            let Some(text) = block(names(subset)) else {
                assert!(advertised < 2, "subset {subset:04b} lost its block");
                continue;
            };
            assert!(advertised >= 2, "subset {subset:04b} got a block");
            let lines = text.lines().filter(|l| l.starts_with("- ")).count();
            assert_eq!(lines, advertised, "one line per family:\n{text}");
            for (index, (marker, _)) in FAMILIES.iter().enumerate() {
                assert_eq!(
                    text.contains(marker),
                    subset & (1 << index) != 0,
                    "{marker} in subset {subset:04b}:\n{text}"
                );
            }
            // No line may name a family that is not on the turn, either.
            assert_eq!(text.contains("jev_browse"), subset & 0b0001 != 0, "{text}");
            assert_eq!(
                text.contains("browser_use_"),
                subset & 0b0100 != 0,
                "{text}"
            );
            assert_eq!(text.contains("webwright"), subset & 0b1000 != 0, "{text}");
            assert_eq!(text.contains("chrome_"), subset & 0b0010 != 0, "{text}");
        }
    }

    #[test]
    fn full_block_is_about_120_words_and_under_150() {
        let text = all_four();
        let words = text.split_whitespace().count();
        assert!((90..150).contains(&words), "{words} words:\n{text}");
    }

    #[test]
    fn block_is_a_pure_function_of_the_advertised_names() {
        let forward = names(0b1111);
        let mut reversed = forward.clone();
        reversed.reverse();
        let mut doubled = forward.clone();
        doubled.extend(forward.iter().copied());
        let first = block(forward.clone()).unwrap();
        assert_eq!(first, block(forward.clone()).unwrap(), "repeat call");
        assert_eq!(first, block(reversed).unwrap(), "tool order");
        assert_eq!(first, block(doubled).unwrap(), "duplicate names");
        // Other tools, and how many tools a family has, change nothing.
        let mut fewer = vec!["jev_browse", "chrome_click", "browser_use_click"];
        fewer.push("webwright.run_script");
        assert_eq!(first, block(fewer).unwrap());
    }

    #[test]
    fn each_line_says_when_to_use_its_family() {
        let text = all_four();
        let jev = text
            .lines()
            .find(|l| l.starts_with("- `jev_browse`"))
            .unwrap();
        assert!(jev.contains("one bounded goal"), "{jev}");
        assert!(jev.contains("public site"), "{jev}");
        assert!(jev.contains("`success_condition`"), "{jev}");
        let chrome = text
            .lines()
            .find(|l| l.starts_with("- `chrome_*`"))
            .unwrap();
        assert!(chrome.contains("signed-in Chrome"), "{chrome}");
        assert!(chrome.contains("debugging"), "{chrome}");
        assert!(chrome.contains("`jev_browse` stopped"), "{chrome}");
        let browser_use = text
            .lines()
            .find(|l| l.starts_with("- `browser_use_*`"))
            .unwrap();
        assert!(browser_use.contains("fresh"), "{browser_use}");
        assert!(browser_use.contains("step-by-step"), "{browser_use}");
        let webwright = text
            .lines()
            .find(|l| l.starts_with("- `webwright.*`"))
            .unwrap();
        assert!(webwright.contains("repeatable"), "{webwright}");
        assert!(webwright.contains("evidence"), "{webwright}");
    }

    #[test]
    fn block_never_promises_a_surface_works() {
        let text = all_four();
        assert!(text.contains("can fail at call time"), "{text}");
        assert!(text.contains("try another family"), "{text}");
        assert!(text.contains("automated access"), "{text}");
        for promise in ["is available", "are available", "always works", "will work"] {
            assert!(!text.contains(promise), "{promise}:\n{text}");
        }
        // Without Jev the Chrome line has nothing to say about it.
        let without_jev = block(names(0b1110)).unwrap();
        assert!(!without_jev.contains("jev_browse"), "{without_jev}");
    }

    #[test]
    fn apply_appends_once_after_existing_developer_text() {
        let bundle = InstructionBundle {
            developer: Some("existing".to_string()),
            ..InstructionBundle::default()
        };
        let applied = apply_browser_routing(bundle.clone(), names(0b0011));
        let developer = applied.developer.as_deref().unwrap();
        assert!(developer.starts_with("existing\n\n## Browser Tool Routing"));
        assert_eq!(developer.matches(HEADING).count(), 1);

        let again = apply_browser_routing(applied.clone(), names(0b1111));
        assert_eq!(again, applied, "an inherited block is not duplicated");

        let empty = apply_browser_routing(InstructionBundle::default(), names(0b0101));
        assert!(empty.developer.as_deref().unwrap().starts_with(HEADING));

        let one = apply_browser_routing(bundle.clone(), names(0b0100));
        assert_eq!(one, bundle, "one family leaves the bundle alone");
    }
}
