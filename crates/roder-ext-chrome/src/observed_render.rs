//! An observation as the model reads it, within the output budget.
//!
//! What the model needs to act comes first: what the last action did, then
//! the controls with their refs, then the page's text. When the whole does not
//! fit, the text is cut first, then forms and frames, and the controls last;
//! whatever is cut says how much of it there was.
//!
//! The budget is in characters and in lines, because the runtime cuts a tool
//! result by both (20,000 characters, 200 lines) and keeps only the two ends of
//! what it cuts: a rendering that stays inside a margin of both is read whole,
//! its outcome line first and its omission lines last.

use crate::observed::{Comparison, Control, Observed, clip, one_line};

/// How much one rendering may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Budget {
    pub(crate) chars: usize,
    pub(crate) lines: usize,
}

impl Budget {
    /// What a page result may take: a margin inside the runtime's own cut of
    /// a tool result.
    pub(crate) const RESULT: Self = Self {
        chars: 18_000,
        lines: 150,
    };
}

/// Characters of the page's address the result shows; the observation keeps
/// more, to tell one page from another.
const PAGE_URL_CHARS: usize = 300;
/// Room kept for the line that says a block was cut.
const MARKER_ROOM: usize = 80;
/// Room kept for the text when the controls take the rest: its header and
/// the marker that says it was withheld, and nothing more, so the controls
/// are only cut when they would not fit beside that.
const TEXT_RESERVE: usize = "Text:\n".len() + MARKER_ROOM + 2;
/// The lines of that: the header and the marker.
const TEXT_RESERVE_LINES: usize = 2;
/// The most a cut block costs besides its header and the lines it keeps: its
/// marker, in lines.
const MARKER_LINES: usize = 1;

/// What is left to spend.
#[derive(Debug, Clone, Copy)]
struct Room {
    chars: usize,
    lines: usize,
}

impl Room {
    fn plus(self, other: Self) -> Self {
        Self {
            chars: self.chars + other.chars,
            lines: self.lines + other.lines,
        }
    }

    /// What a block put after others has for itself: the line break that
    /// joins it to them is not its to spend.
    fn joined(self) -> Self {
        Self {
            chars: self.chars.saturating_sub(1),
            ..self
        }
    }

    fn less(self, reserve: Self) -> Self {
        Self {
            chars: self.chars.saturating_sub(reserve.chars),
            lines: self.lines.saturating_sub(reserve.lines),
        }
    }

    /// What `block` takes of it: its characters, a line break to join it to
    /// what came before, and its lines.
    fn spend(self, block: &str) -> Self {
        self.less(Self {
            chars: block.chars().count() + 1,
            lines: line_count(block),
        })
    }
}

fn line_count(text: &str) -> usize {
    text.matches('\n').count() + 1
}

/// What a rendering shows.
pub(crate) struct View<'a> {
    /// The untrusted-content label that leads every result.
    pub(crate) note: &'a str,
    /// What the action did to the page, when the action is what is shown.
    pub(crate) comparison: Option<&'a Comparison>,
    /// Lines of the tool's own, after the outcome.
    pub(crate) notices: Vec<String>,
    /// What the action itself answered, already on one line.
    pub(crate) action: Option<String>,
    pub(crate) page: &'a Observed,
}

/// The view as text within `budget`.
pub(crate) fn render(view: &View<'_>, budget: Budget) -> String {
    let page = view.page;
    let mut head = vec![view.note.to_string()];
    if let Some(comparison) = view.comparison {
        head.push(format!("Outcome: {}", comparison.sentence));
    }
    head.extend(view.notices.iter().cloned());
    if let Some(action) = &view.action {
        head.push(format!("Action result: {action}"));
    }
    head.push(format!("Page: {}", clip(&page.url, PAGE_URL_CHARS)));
    head.push(format!("Title: {}", page.title));
    if let Some((width, height)) = page.viewport {
        head.push(match page.scroll_y {
            Some(y) => format!("Viewport {width}x{height} px, scrolled to {y}."),
            None => format!("Viewport {width}x{height} px."),
        });
    }
    let head = head.join("\n");
    let mut room = Room {
        chars: budget.chars,
        lines: budget.lines,
    }
    .less(Room {
        chars: head.chars().count(),
        lines: line_count(&head),
    });
    let mut blocks = vec![head];
    let mut withhold_text = false;

    let controls: Vec<String> = page
        .controls
        .iter()
        .map(|control| control_line(control, view.comparison))
        .collect();
    // A section the snapshot was not asked for is empty without the page
    // having none: it is not reported as "none found".
    let header = match (page.read.controls, controls.is_empty()) {
        (false, _) => "Controls: not requested.",
        (true, true) => "Controls: none found.",
        (true, false) => {
            "Controls (ref, role, \"label\"; act by ref, or for a select by the selector shown):"
        }
    };
    let extras = [
        ("Forms:", &page.forms, "forms"),
        ("Frames:", &page.frames, "frames"),
    ];
    // The controls take the room first and are cut last. Each block that
    // comes after keeps the room its own cut form needs (its header and the
    // line that counts what it left out), and so does the text.
    let mut parts = vec![(header, controls, "controls")];
    parts.extend(
        extras
            .into_iter()
            .filter(|(_, lines, _)| !lines.is_empty())
            .map(|(header, lines, noun)| (header, lines.clone(), noun)),
    );
    for (at, (header, lines, noun)) in parts.iter().enumerate() {
        let behind = parts[at + 1..]
            .iter()
            .fold(text_reserve(page), |sum, (header, lines, _)| {
                sum.plus(cut_cost(header, lines))
            });
        let (text, omitted) = block(header, lines, noun, room.less(behind).joined());
        room = room.spend(&text);
        withhold_text |= omitted > 0;
        blocks.push(text);
    }
    if let Some(text) = text_block(page, room.joined(), withhold_text) {
        blocks.push(text);
    }
    clamp(blocks.join("\n"), budget)
}

/// `whole` itself when it is within `budget`, else its first lines that are,
/// then a line that says so. The blocks above keep to the budget; this is a
/// bound that does not depend on their arithmetic being right.
fn clamp(whole: String, budget: Budget) -> String {
    const CUT: &str = "… cut to the output limit";
    if whole.chars().count() <= budget.chars && line_count(&whole) <= budget.lines {
        return whole;
    }
    let chars = budget.chars.saturating_sub(CUT.chars().count() + 1);
    let mut kept: Vec<&str> = Vec::new();
    let mut used = 0;
    for line in whole.lines().take(budget.lines.saturating_sub(1)) {
        let cost = line.chars().count() + 1;
        if used + cost > chars {
            break;
        }
        used += cost;
        kept.push(line);
    }
    kept.push(CUT);
    kept.join("\n")
}

/// What a block takes at its smallest, once it has to be cut: its header and
/// the line that counts what it left out; its header alone when it has no
/// lines to leave out.
fn cut_cost(header: &str, lines: &[String]) -> Room {
    let header = header.chars().count();
    match lines.is_empty() {
        true => Room {
            chars: header + 1,
            lines: 1,
        },
        false => Room {
            chars: header + 1 + MARKER_ROOM + 1,
            lines: 1 + MARKER_LINES,
        },
    }
}

/// What the text takes at its smallest: its header and the line that says
/// how much of it was left out.
fn text_reserve(page: &Observed) -> Room {
    match (page.read.text, page.text.is_empty()) {
        (false, _) => Room {
            chars: "Text: not requested.".len() + 1,
            lines: 1,
        },
        (true, true) => Room { chars: 0, lines: 0 },
        (true, false) => Room {
            chars: TEXT_RESERVE,
            lines: TEXT_RESERVE_LINES,
        },
    }
}

/// A header and as many of `lines` as fit in `room`; when some do not, a
/// last line says how many were left out.
fn block(header: &str, lines: &[String], noun: &str, room: Room) -> (String, usize) {
    let cost = |line: &String| line.chars().count() + 1;
    let all = header.chars().count() + lines.iter().map(cost).sum::<usize>();
    // The header is a line too.
    if all <= room.chars && lines.len() < room.lines {
        let mut out = vec![header.to_string()];
        out.extend(lines.iter().cloned());
        return (out.join("\n"), 0);
    }
    let chars = room.chars.saturating_sub(MARKER_ROOM);
    let line_room = room.lines.saturating_sub(1 + MARKER_LINES);
    let mut used = header.chars().count();
    let mut kept = 0;
    for line in lines {
        if kept == line_room || used + cost(line) > chars {
            break;
        }
        used += cost(line);
        kept += 1;
    }
    let mut out = vec![header.to_string()];
    out.extend(lines[..kept].iter().cloned());
    let omitted = lines.len() - kept;
    out.push(format!("… {omitted} more {noun} not listed"));
    (out.join("\n"), omitted)
}

/// The page's text, indented so that none of it starts a line of the
/// result's own, and a line saying how much of it there was when it is cut.
fn text_block(page: &Observed, room: Room, withhold: bool) -> Option<String> {
    if !page.read.text {
        return Some("Text: not requested.".to_string());
    }
    let total = page.text.chars().count();
    if total == 0 {
        return None;
    }
    let source_cut = page.text_cut;
    let overhead = "Text:\n  ".len() + MARKER_ROOM;
    let mut kept = match withhold {
        true => 0,
        false => total.min(room.chars.saturating_sub(overhead)),
    };
    // The text is one line; its header, and the line that says it was cut,
    // are the others.
    let lines = |kept: usize| 1 + usize::from(kept > 0) + usize::from(kept < total || source_cut);
    if lines(kept) > room.lines {
        kept = 0;
    }
    let mut out = "Text:".to_string();
    if kept > 0 {
        out.push_str(&format!("\n  {}", cut(&page.text, kept)));
    }
    if kept < total {
        let least = if source_cut { "at least " } else { "" };
        out.push_str(&format!(
            "\n… text cut at {kept} chars (the page text is {least}{total} chars)"
        ));
    } else if source_cut {
        out.push_str(&format!(
            "\n… text cut at {total} chars (the extension's limit; the page has more)"
        ));
    }
    Some(out)
}

fn cut(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

/// `c3 checkbox "Agree" [checked] [changed]`.
fn control_line(control: &Control, comparison: Option<&Comparison>) -> String {
    let mut line = format!("{} {}", control.reference, control.role);
    if !control.label.is_empty() {
        line.push_str(&format!(" \"{}\"", control.label));
    }
    if control.secret {
        line.push_str(match control.filled {
            Some(true) => " (secret field, filled)",
            Some(false) => " (secret field, empty)",
            None => " (secret field)",
        });
    } else if let Some(value) = &control.value {
        line.push_str(&format!(" = \"{value}\""));
    }
    match control.checked {
        Some(true) => line.push_str(" [checked]"),
        Some(false) => line.push_str(" [unchecked]"),
        None => {}
    }
    match control.expanded {
        Some(true) => line.push_str(" [expanded]"),
        Some(false) => line.push_str(" [collapsed]"),
        None => {}
    }
    if control.disabled {
        line.push_str(" [disabled]");
    }
    if let Some(selector) = &control.selector {
        line.push_str(&format!(" selector {}", one_line(selector, 80)));
    }
    if let Some(comparison) = comparison {
        if comparison.added.contains(&control.reference) {
            line.push_str(" [new]");
        } else if comparison.altered.contains(&control.reference) {
            line.push_str(" [changed]");
        }
    }
    line
}
