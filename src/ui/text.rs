//! Display-width clipping and footer line composition helpers.

use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::theme::footer_hint;

/// What: Truncates `s` to fit `max_cols` display columns, appending `…` when shortened.
///
/// Inputs:
/// - `s`: Source string, may contain wide characters.
/// - `max_cols`: Maximum number of display columns.
///
/// Output:
/// - Owned string at most `max_cols` columns wide, with `…` appended when truncated.
///
/// Details:
/// - Returns an empty string when `max_cols` is `0`.
/// - When the very first character does not fit the budget, returns just `…`.
pub fn clip_display_width(s: &str, max_cols: u16) -> String {
    let max = usize::from(max_cols);
    if max == 0 {
        return String::new();
    }
    if s.width() <= max {
        return s.to_owned();
    }
    let budget = max.saturating_sub(1);
    let out = take_chars_within_width(s, budget);
    if out.is_empty() {
        "…".to_string()
    } else {
        format!("{out}…")
    }
}

/// Greedily collects characters from `s` while their cumulative display width fits `budget`.
fn take_chars_within_width(s: &str, budget: usize) -> String {
    let mut out = String::new();
    let mut used = 0usize;
    for ch in s.chars() {
        let w = unicode_width::UnicodeWidthChar::width(ch)
            .unwrap_or(0)
            .max(1);
        if used + w > budget {
            break;
        }
        out.push(ch);
        used += w;
    }
    out
}

/// What: Builds a single keybinding row inside a footer column.
///
/// Inputs:
/// - `key_spans`: One or more spans rendered with the key-label style.
/// - `hint`: Trailing description rendered with the muted hint style.
///
/// Output:
/// - A `Line` combining the key spans followed by the hint span.
///
/// Details:
/// - Used by both the main and overlay footer columns to keep formatting uniform.
pub fn footer_col_line<'a, I>(key_spans: I, hint: &'a str) -> Line<'a>
where
    I: IntoIterator<Item = Span<'a>>,
{
    let mut spans: Vec<Span<'a>> = key_spans.into_iter().collect();
    spans.push(footer_hint(hint));
    Line::from(spans)
}

/// What: Builds a line where query matches are highlighted in normal or fuzzy mode.
///
/// Inputs:
/// - `text`: Source text to render.
/// - `query`: Search query used to find matches; empty query leaves text unmodified.
/// - `fuzzy`: When true, highlights ordered non-contiguous query character matches.
/// - `base`: Style used for non-matching segments.
/// - `highlight`: Style used for matching segments.
///
/// Output:
/// - A `Line` composed of spans that alternate between `base` and `highlight`.
///
/// Details:
/// - Matching is case-insensitive for ASCII characters.
/// - In normal mode, non-overlapping substring ranges are highlighted.
/// - In fuzzy mode, individual matched characters are highlighted in order.
/// - This helper is intended for package-name highlighting while search mode is active.
pub fn highlight_ascii_matches<'a>(
    text: &'a str,
    query: &str,
    fuzzy: bool,
    base: ratatui::style::Style,
    highlight: ratatui::style::Style,
) -> Line<'a> {
    if query.is_empty() {
        return Line::from(Span::styled(text, base));
    }
    let ranges = if fuzzy {
        fuzzy_highlight_ranges(text, query)
    } else {
        substring_highlight_ranges(text, query)
    };
    if ranges.is_empty() {
        return Line::from(Span::styled(text, base));
    }

    build_line_from_ranges(text, &ranges, base, highlight)
}

/// Returns non-overlapping byte ranges for case-insensitive ASCII substring matches.
fn substring_highlight_ranges(text: &str, query: &str) -> Vec<(usize, usize)> {
    let haystack = text.to_ascii_lowercase();
    let needle = query.to_ascii_lowercase();
    if needle.is_empty() || !haystack.contains(&needle) {
        return Vec::new();
    }
    let mut ranges = Vec::new();
    let mut search_from = 0usize;
    while let Some(found_rel) = haystack[search_from..].find(&needle) {
        let start = search_from + found_rel;
        let end = start + needle.len();
        ranges.push((start, end));
        search_from = end;
    }
    ranges
}

/// Returns per-character byte ranges for ordered fuzzy subsequence matches.
fn fuzzy_highlight_ranges(text: &str, query: &str) -> Vec<(usize, usize)> {
    let mut needle = query.chars().map(|c| c.to_ascii_lowercase());
    let Some(mut current) = needle.next() else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for (start, ch) in text.char_indices() {
        if ch.to_ascii_lowercase() == current {
            let end = start + ch.len_utf8();
            out.push((start, end));
            if let Some(next) = needle.next() {
                current = next;
            } else {
                return out;
            }
        }
    }
    Vec::new()
}

/// Builds a line from `text` by highlighting the provided non-overlapping byte ranges.
fn build_line_from_ranges<'a>(
    text: &'a str,
    ranges: &[(usize, usize)],
    base: ratatui::style::Style,
    highlight: ratatui::style::Style,
) -> Line<'a> {
    let mut spans = Vec::new();
    let mut cursor = 0usize;
    for &(start, end) in ranges {
        if start > cursor {
            spans.push(Span::styled(&text[cursor..start], base));
        }
        spans.push(Span::styled(&text[start..end], highlight));
        cursor = end;
    }
    if cursor < text.len() {
        spans.push(Span::styled(&text[cursor..], base));
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use ratatui::style::Style;

    use super::highlight_ascii_matches;

    #[test]
    fn highlight_ascii_matches_marks_every_non_overlapping_match() {
        let line =
            highlight_ascii_matches("nanana", "na", false, Style::default(), Style::default());
        let rendered = line
            .spans
            .into_iter()
            .map(|s| s.content)
            .collect::<Vec<_>>();
        assert_eq!(rendered, vec!["na", "na", "na"]);
    }

    #[test]
    fn highlight_ascii_matches_is_case_insensitive_for_ascii() {
        let line = highlight_ascii_matches(
            "SerdeJson",
            "json",
            false,
            Style::default(),
            Style::default(),
        );
        let rendered = line
            .spans
            .into_iter()
            .map(|s| s.content)
            .collect::<Vec<_>>();
        assert_eq!(rendered, vec!["Serde", "Json"]);
    }

    #[test]
    fn highlight_ascii_matches_highlights_non_contiguous_fuzzy_matches() {
        let line =
            highlight_ascii_matches("neovim", "nvm", true, Style::default(), Style::default());
        let rendered = line
            .spans
            .into_iter()
            .map(|s| s.content)
            .collect::<Vec<_>>();
        assert_eq!(rendered, vec!["n", "eo", "v", "i", "m"]);
    }
}
