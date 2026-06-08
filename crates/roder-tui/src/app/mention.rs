//! Inline `@file` and `$skill` autocomplete for the composer.
//!
//! Typing `@` opens a fuzzy file picker over the workspace; selecting a file
//! inserts an `@<path>` token the agent can read. Typing `$` opens a fuzzy
//! skill picker; selecting a skill inserts a `$<name>` token, which the
//! backend resolves into a direct skill invocation
//! (`roder_skills::parse_skill_invocations`).
//!
//! Both popups mirror the existing slash-command menu: they live below the
//! composer, navigate with Up/Down, accept with Tab/Enter, and close with Esc.

use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ignore::WalkBuilder;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use roder_api::skills::SkillDescriptor;

use super::{AppClient, Theme, TuiApp};

impl<C> TuiApp<C>
where
    C: AppClient,
{
    /// Re-derive the mention popup from the cursor position after a composer
    /// edit. Lazily loads the file/skill catalog the first time each picker is
    /// opened, and closes the popup when the cursor leaves a mention token.
    pub(super) async fn update_mention_popup(&mut self) {
        let Some(token) = self.current_mention_token() else {
            self.mention.popup = None;
            return;
        };
        match token.kind {
            MentionKind::File => {
                if self.mention.file_cache.is_none() {
                    let root =
                        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                    self.mention.file_cache = Some(walk_workspace_files(&root));
                }
            }
            MentionKind::Skill => {
                if self.mention.skills.is_empty()
                    && let Ok(result) = self.skills_list().await
                {
                    self.mention.skills = skill_candidates(&result.skills);
                }
            }
        }
        match self.mention.popup.as_mut() {
            Some(popup) if popup.kind == token.kind => {
                if popup.query != token.query {
                    popup.query = token.query;
                    popup.selection = 0;
                }
            }
            _ => {
                self.mention.popup = Some(MentionPopup {
                    kind: token.kind,
                    query: token.query,
                    selection: 0,
                });
            }
        }
    }

    fn current_mention_token(&self) -> Option<MentionToken> {
        let (row, col) = self.composer.cursor();
        let line = self.composer.lines().get(row)?;
        active_mention_token(line, col)
    }

    /// Handle navigation/accept/dismiss keys while a mention popup is open.
    /// Returns `true` when the key was consumed; character edits fall through so
    /// the composer updates and [`Self::update_mention_popup`] re-filters.
    pub(super) async fn handle_mention_key(&mut self, key: KeyEvent) -> bool {
        if self.mention.popup.is_none() {
            return false;
        }
        match key.code {
            KeyCode::Esc => {
                self.mention.popup = None;
                true
            }
            KeyCode::Up => {
                self.move_mention_selection(-1);
                true
            }
            KeyCode::Down => {
                self.move_mention_selection(1);
                true
            }
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_mention_selection(-1);
                true
            }
            KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_mention_selection(1);
                true
            }
            KeyCode::Tab | KeyCode::Enter => {
                self.accept_mention();
                true
            }
            _ => false,
        }
    }

    fn move_mention_selection(&mut self, delta: isize) {
        let count = self
            .mention_matches()
            .map(|(_, matches, _)| matches.len())
            .unwrap_or(0);
        let Some(popup) = self.mention.popup.as_mut() else {
            return;
        };
        if count == 0 {
            popup.selection = 0;
            return;
        }
        popup.selection = (popup.selection as isize + delta).rem_euclid(count as isize) as usize;
    }

    /// Replace the active `@`/`$` token with the selected entry and a trailing
    /// space, then close the popup.
    fn accept_mention(&mut self) {
        let Some((_, matches, selection)) = self.mention_matches() else {
            self.mention.popup = None;
            return;
        };
        let (Some(matched), Some(token)) = (
            matches.get(selection).cloned(),
            self.current_mention_token(),
        ) else {
            self.mention.popup = None;
            return;
        };
        // The cursor sits at the end of the token, so the sigil and query are
        // the `query.len() + 1` characters immediately behind it.
        let to_delete = token.query.chars().count() + 1;
        for _ in 0..to_delete {
            self.composer.delete_char();
        }
        self.composer.insert_str(format!("{} ", matched.insert));
        self.mention.popup = None;
    }

    /// Current popup kind, ranked matches, and clamped selection, or `None` when
    /// no popup is active, the cursor has left the token, or nothing matches.
    pub(super) fn mention_matches(&self) -> Option<(MentionKind, Vec<MentionMatch>, usize)> {
        let popup = self.mention.popup.as_ref()?;
        let token = self.current_mention_token()?;
        if token.kind != popup.kind {
            return None;
        }
        let matches = match popup.kind {
            MentionKind::File => {
                let files = self.mention.file_cache.as_deref().unwrap_or(&[]);
                file_matches(files, &popup.query)
            }
            MentionKind::Skill => skill_matches(&self.mention.skills, &popup.query),
        };
        if matches.is_empty() {
            return None;
        }
        let selection = popup.selection.min(matches.len() - 1);
        Some((popup.kind, matches, selection))
    }
}

/// Maximum rows shown in a mention popup.
pub(super) const MAX_VISIBLE_MENTIONS: usize = 12;
/// Upper bound on workspace files scanned for the `@` picker. Keeps the walk
/// bounded on very large trees; truncation is silent because the fuzzy filter
/// surfaces the relevant matches regardless of catalog order.
const MAX_WORKSPACE_FILES: usize = 20_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MentionKind {
    File,
    Skill,
}

impl MentionKind {
    fn sigil(self) -> char {
        match self {
            Self::File => '@',
            Self::Skill => '$',
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::File => "Files",
            Self::Skill => "Skills",
        }
    }
}

/// A live mention picker anchored to a token in the composer.
#[derive(Clone, Debug)]
pub(super) struct MentionPopup {
    pub(super) kind: MentionKind,
    pub(super) query: String,
    pub(super) selection: usize,
}

/// One row offered by a mention popup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MentionMatch {
    /// Text inserted into the composer (without the trailing space).
    pub(super) insert: String,
    /// Primary label shown in the popup.
    pub(super) title: String,
    /// Optional secondary description.
    pub(super) subtitle: Option<String>,
}

/// Composer-mention state bundled so the two `TuiApp` constructors only need a
/// single `MentionState::default()` field initializer.
#[derive(Default)]
pub(super) struct MentionState {
    pub(super) popup: Option<MentionPopup>,
    /// Workspace-relative file paths, walked lazily the first time `@` is used.
    pub(super) file_cache: Option<Vec<String>>,
    /// Skill candidates, populated lazily the first time `$` is used.
    pub(super) skills: Vec<MentionCandidate>,
}

/// A pre-rendered candidate kept in the skill cache.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MentionCandidate {
    pub(super) insert: String,
    pub(super) title: String,
    pub(super) subtitle: Option<String>,
    /// Lowercased text searched by the fuzzy filter.
    haystacks: Vec<String>,
}

/// The mention token under the cursor, if the cursor sits at the end of a
/// `@`/`$` token whose sigil is preceded by whitespace or start-of-line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MentionToken {
    pub(super) kind: MentionKind,
    pub(super) query: String,
}

fn is_token_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/' | ':')
}

/// Detect a mention token on `line` with the cursor at character column `col`.
///
/// Returns `Some` only when the cursor is at the end of the token (the next
/// character, if any, is not a token character), the run is introduced by an
/// `@` or `$` sigil, and the sigil itself is at start-of-line or preceded by
/// whitespace. This deliberately avoids triggering inside `foo@bar` emails or
/// mid-word `$` usage.
pub(super) fn active_mention_token(line: &str, col: usize) -> Option<MentionToken> {
    let chars: Vec<char> = line.chars().collect();
    if col > chars.len() {
        return None;
    }
    // Cursor must sit at the end of the token, not inside it.
    if col < chars.len() && is_token_char(chars[col]) {
        return None;
    }
    let mut start = col;
    while start > 0 && is_token_char(chars[start - 1]) {
        start -= 1;
    }
    if start == 0 {
        return None;
    }
    let kind = match chars[start - 1] {
        '@' => MentionKind::File,
        '$' => MentionKind::Skill,
        _ => return None,
    };
    // The sigil must begin a word (start-of-line or after whitespace).
    if start >= 2 && !chars[start - 2].is_whitespace() {
        return None;
    }
    let query: String = chars[start..col].iter().collect();
    // Skill names never contain path separators; bail so `$a/b` does not open a
    // skill popup that can never match.
    if kind == MentionKind::Skill && query.contains('/') {
        return None;
    }
    Some(MentionToken { kind, query })
}

/// Build the skill cache from a catalog. Skills are offered regardless of their
/// enabled state because a direct `$name` invocation activates them per turn.
pub(super) fn skill_candidates(skills: &[SkillDescriptor]) -> Vec<MentionCandidate> {
    let mut candidates: Vec<MentionCandidate> = skills
        .iter()
        .map(|skill| {
            let subtitle = skill
                .short_description
                .clone()
                .filter(|text| !text.trim().is_empty())
                .or_else(|| {
                    let trimmed = skill.description.trim();
                    (!trimmed.is_empty()).then(|| trimmed.to_string())
                });
            let mut haystacks = vec![skill.name.to_ascii_lowercase()];
            if let Some(subtitle) = &subtitle {
                haystacks.push(subtitle.to_ascii_lowercase());
            }
            MentionCandidate {
                insert: skill_insert_text(&skill.name),
                title: skill.name.clone(),
                subtitle,
                haystacks,
            }
        })
        .collect();
    candidates.sort_by(|a, b| a.title.cmp(&b.title));
    candidates.dedup_by(|a, b| a.title == b.title);
    candidates
}

/// Insert text for a skill mention. Simple names use the bare `$name` form the
/// invocation parser prefers; names with other characters fall back to the
/// braced `${name}` form.
fn skill_insert_text(name: &str) -> String {
    let simple = !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');
    if simple {
        format!("${name}")
    } else {
        format!("${{{name}}}")
    }
}

/// Walk the workspace rooted at `root`, returning workspace-relative file paths.
/// Honors `.gitignore` and skips hidden files and `.git` via [`WalkBuilder`].
pub(super) fn walk_workspace_files(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    let walker = WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .parents(true)
        .build();
    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let Ok(relative) = entry.path().strip_prefix(root) else {
            continue;
        };
        let display = relative.to_string_lossy();
        if display.is_empty() {
            continue;
        }
        files.push(display.replace('\\', "/"));
        if files.len() >= MAX_WORKSPACE_FILES {
            break;
        }
    }
    files.sort();
    files
}

/// Filtered, ranked file matches for `query`. Scores the full path and the
/// basename (with a bonus) so `cargo` ranks `Cargo.toml` above a deep path that
/// merely contains the substring.
pub(super) fn file_matches(files: &[String], query: &str) -> Vec<MentionMatch> {
    let query = query.trim();
    let mut scored: Vec<(i32, &String)> = files
        .iter()
        .filter_map(|path| file_score(path, query).map(|score| (score, path)))
        .collect();
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.len().cmp(&b.1.len()))
            .then_with(|| a.1.cmp(b.1))
    });
    scored
        .into_iter()
        .take(MAX_VISIBLE_MENTIONS)
        .map(|(_, path)| MentionMatch {
            insert: format!("@{path}"),
            title: path.clone(),
            subtitle: None,
        })
        .collect()
}

fn file_score(path: &str, query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let basename = path.rsplit('/').next().unwrap_or(path);
    let mut best = score_text(path, query);
    if let Some(name_score) = score_text(basename, query) {
        best = Some(best.map_or(name_score + 40, |current| current.max(name_score + 40)));
    }
    best
}

/// Filtered, ranked skill matches for `query`.
pub(super) fn skill_matches(candidates: &[MentionCandidate], query: &str) -> Vec<MentionMatch> {
    let query = query.trim();
    let mut scored: Vec<(i32, &MentionCandidate)> = candidates
        .iter()
        .filter_map(|candidate| candidate_score(candidate, query).map(|score| (score, candidate)))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.title.cmp(&b.1.title)));
    scored
        .into_iter()
        .take(MAX_VISIBLE_MENTIONS)
        .map(|(_, candidate)| MentionMatch {
            insert: candidate.insert.clone(),
            title: candidate.title.clone(),
            subtitle: candidate.subtitle.clone(),
        })
        .collect()
}

fn candidate_score(candidate: &MentionCandidate, query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    // Name match (first haystack) outranks description matches.
    let mut best = candidate
        .haystacks
        .first()
        .and_then(|name| score_text(name, query).map(|score| score + 40));
    for haystack in candidate.haystacks.iter().skip(1) {
        if let Some(score) = score_text(haystack, query) {
            best = Some(best.map_or(score, |current| current.max(score)));
        }
    }
    best
}

/// Subsequence-aware fuzzy score shared by both pickers. Mirrors the palette
/// scorer: exact > prefix > substring > scattered subsequence.
fn score_text(text: &str, query: &str) -> Option<i32> {
    let text = text.to_ascii_lowercase();
    let query = query.to_ascii_lowercase();
    if text == query {
        return Some(300);
    }
    if text.starts_with(&query) {
        return Some(240 - text.len() as i32);
    }
    if let Some(index) = text.find(&query) {
        return Some(180 - index as i32);
    }
    fuzzy_subsequence_score(&text, &query)
}

fn fuzzy_subsequence_score(text: &str, query: &str) -> Option<i32> {
    let mut score = 90;
    let mut last_match: Option<usize> = None;
    let mut chars = text.char_indices();
    for query_char in query.chars() {
        let (index, _) = chars.find(|(_, text_char)| *text_char == query_char)?;
        if let Some(last) = last_match {
            score -= (index - last - 1).min(12) as i32;
        } else {
            score -= index.min(12) as i32;
        }
        last_match = Some(index);
    }
    Some(score)
}

/// Height of the mention menu, including its header row.
pub(super) fn mention_menu_height(matches: Option<&[MentionMatch]>) -> u16 {
    match matches {
        Some(matches) if !matches.is_empty() => 1 + matches.len().min(MAX_VISIBLE_MENTIONS) as u16,
        _ => 0,
    }
}

/// Render the mention menu widget. `selection` is clamped by the caller.
pub(super) fn mention_menu(
    kind: MentionKind,
    matches: &[MentionMatch],
    selection: usize,
    theme: Theme,
) -> Paragraph<'static> {
    if matches.is_empty() {
        return Paragraph::new(Text::default());
    }
    let selected_index = selection.min(matches.len().saturating_sub(1));
    let mut lines = Vec::with_capacity(matches.len() + 1);
    lines.push(Line::from(vec![
        Span::styled(format!(" {} ", kind.title()), theme.strong()),
        Span::styled(
            format!("{} mention  tab/enter insert  up/down select", kind.sigil()),
            theme.subtle(),
        ),
    ]));
    for (index, matched) in matches.iter().take(MAX_VISIBLE_MENTIONS).enumerate() {
        let selected = index == selected_index;
        let marker = if selected { ">" } else { " " };
        let style = if selected {
            theme.selected()
        } else {
            theme.text()
        };
        let mut spans = vec![
            Span::styled(format!(" {marker} "), theme.subtle()),
            Span::styled(matched.title.clone(), style),
        ];
        if let Some(subtitle) = &matched.subtitle {
            spans.push(Span::styled(
                format!("  {}", truncate(subtitle, 64)),
                theme.muted(),
            ));
        }
        lines.push(Line::from(spans));
    }
    Paragraph::new(Text::from(lines)).style(theme.text())
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::skills::{SkillActivationState, SkillExposure, SkillSource};

    fn skill(name: &str, description: &str) -> SkillDescriptor {
        SkillDescriptor {
            id: name.to_string(),
            name: name.to_string(),
            canonical_path: format!("test://{name}/SKILL.md"),
            source: SkillSource::Workspace,
            exposure: SkillExposure::Global,
            activation: SkillActivationState::Enabled,
            description: description.to_string(),
            short_description: None,
            experimental: false,
            diagnostics: Vec::new(),
            agent_metadata: None,
        }
    }

    #[test]
    fn detects_file_token_at_cursor_end() {
        let token = active_mention_token("review @src/ma", 14).unwrap();
        assert_eq!(token.kind, MentionKind::File);
        assert_eq!(token.query, "src/ma");
    }

    #[test]
    fn detects_skill_token_at_start_of_line() {
        let token = active_mention_token("$rev", 4).unwrap();
        assert_eq!(token.kind, MentionKind::Skill);
        assert_eq!(token.query, "rev");
    }

    #[test]
    fn empty_token_opens_picker_immediately() {
        let token = active_mention_token("look at @", 9).unwrap();
        assert_eq!(token.kind, MentionKind::File);
        assert_eq!(token.query, "");
    }

    #[test]
    fn ignores_email_like_at_signs() {
        assert_eq!(active_mention_token("mail me at foo@bar", 18), None);
    }

    #[test]
    fn ignores_cursor_inside_token() {
        // Cursor before the trailing token chars -> not at the token end.
        assert_eq!(active_mention_token("@src/main", 4), None);
    }

    #[test]
    fn skill_token_rejects_path_separators() {
        assert_eq!(active_mention_token("$a/b", 4), None);
    }

    #[test]
    fn file_matches_rank_basename_hits_first() {
        let files = vec![
            "deep/nested/cargo_helpers.rs".to_string(),
            "Cargo.toml".to_string(),
        ];
        let matches = file_matches(&files, "cargo");
        assert_eq!(matches[0].insert, "@Cargo.toml");
        assert_eq!(matches[0].title, "Cargo.toml");
    }

    #[test]
    fn skill_matches_filter_and_insert_dollar_token() {
        let candidates = skill_candidates(&[
            skill("review-environments", "Review verifiers environments"),
            skill("deep-research", "Research harness"),
        ]);
        let matches = skill_matches(&candidates, "review");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].insert, "$review-environments");
        assert_eq!(
            matches[0].subtitle.as_deref(),
            Some("Review verifiers environments")
        );
    }

    #[test]
    fn namespaced_skill_uses_braced_insert() {
        let candidates = skill_candidates(&[skill("anthropic-skills:docx", "Word docs")]);
        assert_eq!(candidates[0].insert, "${anthropic-skills:docx}");
    }

    #[test]
    fn empty_query_lists_all_candidates() {
        let candidates = skill_candidates(&[skill("a", ""), skill("b", "")]);
        assert_eq!(skill_matches(&candidates, "").len(), 2);
    }

    #[test]
    fn menu_height_includes_header_row() {
        let matches = vec![MentionMatch {
            insert: "@a".to_string(),
            title: "a".to_string(),
            subtitle: None,
        }];
        assert_eq!(mention_menu_height(Some(&matches)), 2);
        assert_eq!(mention_menu_height(Some(&[])), 0);
        assert_eq!(mention_menu_height(None), 0);
    }
}
