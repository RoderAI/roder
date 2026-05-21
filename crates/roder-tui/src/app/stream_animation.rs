use std::time::{Duration, Instant};

use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

use super::Theme;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) enum StreamFadePalette {
    Accent,
    Neutral,
}

/// Default time budget for a calm one-line reveal. Tune this to make streamed
/// assistant text feel slower or faster without touching the catch-up policy.
pub(super) const STREAM_ANIMATION_TIME: Duration = Duration::from_millis(620);
pub(super) const STREAM_ANIMATION_FRAME_TIME: Duration = Duration::from_millis(33);

const STREAM_MIN_LINE_ANIMATION_TIME: Duration = Duration::from_millis(380);
const STREAM_CATCH_UP_BUFFER_CHARS: usize = 240;
const STREAM_BURST_BUFFER_CHARS: usize = 960;
const STREAM_GRADIENT_TRAIL_CHARS: usize = 14;

#[derive(Debug, Clone, Eq, PartialEq)]
pub(super) struct AnimatedText {
    visible: String,
    pending: String,
    gradient_len: usize,
}

impl AnimatedText {
    fn new(visible: String, pending: String, gradient_len: usize) -> Self {
        Self {
            visible,
            pending,
            gradient_len,
        }
    }

    pub(super) fn from_visible(visible: String) -> Self {
        Self::new(visible, String::new(), 0)
    }

    pub(super) fn as_str(&self) -> &str {
        &self.visible
    }

    pub(super) fn is_animating(&self) -> bool {
        !self.pending.is_empty() || self.gradient_len > 0
    }
}

#[derive(Debug, Clone)]
pub(super) struct StreamAnimator {
    full_text: String,
    target_visible_chars: usize,
    rendered_visible_chars: usize,
    last_tick: Option<Instant>,
    last_delta_at: Option<Instant>,
    char_credit: f32,
    gradient_remaining: usize,
}

impl Default for StreamAnimator {
    fn default() -> Self {
        Self {
            full_text: String::new(),
            target_visible_chars: 0,
            rendered_visible_chars: 0,
            last_tick: None,
            last_delta_at: None,
            char_credit: 0.0,
            gradient_remaining: 0,
        }
    }
}

impl StreamAnimator {
    pub(super) fn push_delta(&mut self, delta: &str, now: Instant) {
        if delta.is_empty() {
            return;
        }
        self.full_text.push_str(delta);
        self.target_visible_chars = self.full_text.chars().count();
        self.last_delta_at = Some(now);
        if self.last_tick.is_none() {
            self.last_tick = Some(now);
        }
    }

    pub(super) fn set_full_text(&mut self, text: String) {
        self.full_text = text;
        self.target_visible_chars = self.full_text.chars().count();
        self.rendered_visible_chars = self.target_visible_chars;
        self.last_tick = None;
        self.last_delta_at = None;
        self.char_credit = 0.0;
        self.gradient_remaining = 0;
    }

    pub(super) fn sync_to_text(&mut self, text: &str) {
        if self.full_text == text {
            return;
        }
        self.set_full_text(text.to_string());
    }

    pub(super) fn flush(&mut self) -> bool {
        let was_animating = self.is_animating();
        self.rendered_visible_chars = self.target_visible_chars;
        self.char_credit = 0.0;
        self.gradient_remaining = 0;
        self.last_tick = None;
        was_animating
    }

    pub(super) fn tick(&mut self, now: Instant, width: u16) -> bool {
        let previous_visible = self.rendered_visible_chars;
        let previous_gradient = self.gradient_remaining;

        if self.rendered_visible_chars < self.target_visible_chars {
            let elapsed = self
                .last_tick
                .map(|last| now.saturating_duration_since(last))
                .unwrap_or_default();
            self.last_tick = Some(now);
            let rate = chars_per_second(self.buffered_chars(), width);
            self.char_credit += elapsed.as_secs_f32() * rate;
            let mut reveal = self.char_credit.floor() as usize;
            if reveal == 0
                && elapsed > Duration::ZERO
                && self.last_delta_at.is_some_and(|last_delta| {
                    now.saturating_duration_since(last_delta) >= STREAM_ANIMATION_TIME
                })
            {
                reveal = 1;
            }
            let remaining = self.target_visible_chars - self.rendered_visible_chars;
            let reveal = reveal.min(remaining);
            if reveal > 0 {
                self.rendered_visible_chars += reveal;
                self.char_credit -= reveal as f32;
                self.gradient_remaining = STREAM_GRADIENT_TRAIL_CHARS;
            }
        } else {
            self.last_tick = Some(now);
            self.char_credit = 0.0;
            self.gradient_remaining = self.gradient_remaining.saturating_sub(1);
        }

        previous_visible != self.rendered_visible_chars
            || previous_gradient != self.gradient_remaining
    }

    pub(super) fn rendered_text(&self) -> AnimatedText {
        let visible = take_chars(&self.full_text, self.rendered_visible_chars);
        let pending = skip_chars(&self.full_text, self.rendered_visible_chars);
        let gradient_len = self
            .gradient_remaining
            .min(visible.chars().count())
            .min(STREAM_GRADIENT_TRAIL_CHARS);
        AnimatedText::new(visible, pending, gradient_len)
    }

    pub(super) fn is_animating(&self) -> bool {
        self.rendered_visible_chars < self.target_visible_chars || self.gradient_remaining > 0
    }

    fn buffered_chars(&self) -> usize {
        self.target_visible_chars
            .saturating_sub(self.rendered_visible_chars)
    }
}

pub(super) fn animated_markdown_lines(
    rendered: &AnimatedText,
    base_style: Style,
    theme: Theme,
    palette: StreamFadePalette,
    markdown: impl FnOnce(&str, Style, Theme) -> Vec<Line<'static>>,
) -> Vec<Line<'static>> {
    let mut lines = markdown(rendered.as_str(), base_style, theme);
    if rendered.gradient_len > 0 {
        apply_gradient_to_tail(&mut lines, rendered.gradient_len, theme, palette);
    }
    lines
}

pub(super) fn animated_plain_lines(
    rendered: &AnimatedText,
    base_style: Style,
    theme: Theme,
    palette: StreamFadePalette,
) -> Vec<Line<'static>> {
    let mut lines = rendered
        .as_str()
        .split('\n')
        .map(|line| Line::from(Span::styled(line.to_string(), base_style)))
        .collect::<Vec<_>>();
    if rendered.gradient_len > 0 {
        apply_gradient_to_tail(&mut lines, rendered.gradient_len, theme, palette);
    }
    lines
}

fn chars_per_second(buffered_chars: usize, width: u16) -> f32 {
    let line_chars = usize::from(width).max(1);
    let minimum_line_rate = line_chars as f32 / STREAM_MIN_LINE_ANIMATION_TIME.as_secs_f32();
    let base_rate = line_chars as f32 / STREAM_ANIMATION_TIME.as_secs_f32();
    if buffered_chars >= STREAM_BURST_BUFFER_CHARS {
        minimum_line_rate * 4.0
    } else if buffered_chars >= STREAM_CATCH_UP_BUFFER_CHARS {
        minimum_line_rate * 2.0
    } else {
        base_rate.max(1.0)
    }
}

fn take_chars(text: &str, count: usize) -> String {
    text.chars().take(count).collect()
}

fn skip_chars(text: &str, count: usize) -> String {
    text.chars().skip(count).collect()
}

fn apply_gradient_to_tail(
    lines: &mut [Line<'static>],
    mut chars_left: usize,
    theme: Theme,
    palette: StreamFadePalette,
) {
    for line in lines.iter_mut().rev() {
        if chars_left == 0 {
            break;
        }
        let (line_spans, remaining) = gradient_line_tail(&line.spans, chars_left, theme, palette);
        line.spans = line_spans;
        chars_left = remaining;
    }
}

fn gradient_line_tail(
    spans: &[Span<'static>],
    chars_left: usize,
    theme: Theme,
    palette: StreamFadePalette,
) -> (Vec<Span<'static>>, usize) {
    let line_chars = spans
        .iter()
        .map(|span| span.content.chars().count())
        .sum::<usize>();
    if line_chars == 0 {
        return (spans.to_vec(), chars_left);
    }
    let gradient_chars = chars_left.min(line_chars);
    let split_at = line_chars.saturating_sub(gradient_chars);
    let mut seen = 0usize;
    let mut out = Vec::new();

    for span in spans {
        let span_chars = span.content.chars().count();
        let span_start = seen;
        let span_end = seen + span_chars;
        if span_end <= split_at {
            out.push(span.clone());
        } else if span_start >= split_at {
            push_gradient_chars(
                &mut out,
                span.content.as_ref(),
                span.style,
                span_start.saturating_sub(split_at),
                theme,
                palette,
            );
        } else {
            let plain_len = split_at - span_start;
            let plain = span.content.chars().take(plain_len).collect::<String>();
            if !plain.is_empty() {
                out.push(Span::styled(plain, span.style));
            }
            let gradient = span.content.chars().skip(plain_len).collect::<String>();
            push_gradient_chars(&mut out, &gradient, span.style, 0, theme, palette);
        }
        seen = span_end;
    }

    (out, chars_left - gradient_chars)
}

fn push_gradient_chars(
    spans: &mut Vec<Span<'static>>,
    text: &str,
    base_style: Style,
    start_age: usize,
    theme: Theme,
    palette: StreamFadePalette,
) {
    for (offset, ch) in text.chars().enumerate() {
        let style = base_style.patch(Style::default().fg(stream_gradient_color(
            start_age + offset,
            theme,
            palette,
        )));
        spans.push(Span::styled(ch.to_string(), style));
    }
}

fn stream_gradient_color(age: usize, theme: Theme, palette: StreamFadePalette) -> Color {
    match palette {
        StreamFadePalette::Accent => match age {
            0..=2 => theme.accent,
            3..=5 => theme.accent_soft,
            6..=9 => theme.text,
            _ => theme.muted,
        },
        StreamFadePalette::Neutral => match age {
            0..=4 => theme.text,
            5..=9 => theme.muted,
            _ => theme.subtle,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_animator_reveals_characters_over_time() {
        let start = Instant::now();
        let mut animator = StreamAnimator::default();
        animator.push_delta("hello", start);

        assert_eq!(animator.rendered_text().as_str(), "");
        assert!(animator.tick(start + STREAM_ANIMATION_TIME, 80));
        assert_eq!(animator.rendered_text().as_str(), "hello");
    }

    #[test]
    fn stream_animator_catches_up_when_buffer_is_large() {
        let start = Instant::now();
        let mut slow = StreamAnimator::default();
        slow.push_delta(&"x".repeat(10), start);
        slow.tick(start + Duration::from_millis(100), 80);
        let slow_visible = slow.rendered_text().as_str().chars().count();

        let mut fast = StreamAnimator::default();
        fast.push_delta(&"x".repeat(STREAM_BURST_BUFFER_CHARS + 10), start);
        fast.tick(start + Duration::from_millis(100), 80);
        let fast_visible = fast.rendered_text().as_str().chars().count();

        assert!(fast_visible > slow_visible);
    }

    #[test]
    fn stream_animator_rate_is_width_aware() {
        let start = Instant::now();
        let mut narrow = StreamAnimator::default();
        narrow.push_delta(&"x".repeat(500), start);
        narrow.tick(start + Duration::from_millis(100), 20);

        let mut wide = StreamAnimator::default();
        wide.push_delta(&"x".repeat(500), start);
        wide.tick(start + Duration::from_millis(100), 120);

        assert!(
            wide.rendered_text().as_str().chars().count()
                > narrow.rendered_text().as_str().chars().count()
        );
    }

    #[test]
    fn animated_markdown_applies_tail_gradient_without_revealing_pending_text() {
        let theme = Theme::for_dark_background(true);
        let rendered = AnimatedText::new("hello".to_string(), " world".to_string(), 3);
        let lines = animated_markdown_lines(
            &rendered,
            theme.text(),
            theme,
            StreamFadePalette::Accent,
            |body, style, _| vec![Line::from(Span::styled(body.to_string(), style))],
        );

        let line = &lines[0];
        assert_eq!(
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>(),
            "hello"
        );
        assert!(
            line.spans
                .iter()
                .any(|span| span.style.fg == Some(theme.accent))
        );
        assert!(rendered.is_animating());
    }

    #[test]
    fn neutral_stream_fade_avoids_accent_colors() {
        let theme = Theme::for_dark_background(true);
        let rendered = AnimatedText::new("thinking".to_string(), String::new(), 8);
        let lines =
            animated_plain_lines(&rendered, theme.muted(), theme, StreamFadePalette::Neutral);

        let colors = lines[0]
            .spans
            .iter()
            .filter_map(|span| span.style.fg)
            .collect::<Vec<_>>();
        assert!(!colors.contains(&theme.accent));
        assert!(!colors.contains(&theme.accent_soft));
        assert!(colors.contains(&theme.text));
        assert!(colors.contains(&theme.muted));
    }
}
