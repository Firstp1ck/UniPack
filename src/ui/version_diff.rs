//! LCS-based version diff highlighting between installed and latest versions.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use crate::model::Package;

/// Highlight for characters that differ in the installed version.
const DIFF_VERSION_RED: Color = Color::Rgb(247, 118, 142);
/// Highlight for characters that differ in the available version.
const DIFF_VERSION_GREEN: Color = Color::Rgb(158, 206, 106);

/// What: Computes per-character "matched" flags for `old` and `new` via LCS.
///
/// Inputs:
/// - `old`: Installed version string.
/// - `new`: Latest version string.
///
/// Output:
/// - Two boolean vectors, one per source string, where `true` marks characters that participate
///   in the longest-common-subsequence alignment (i.e. unchanged characters).
///
/// Details:
/// - Standard `O(n*m)` LCS dynamic programming followed by backtracking.
fn lcs_char_match_flags(old: &str, new: &str) -> (Vec<bool>, Vec<bool>) {
    let oldc: Vec<char> = old.chars().collect();
    let newc: Vec<char> = new.chars().collect();
    let n = oldc.len();
    let m = newc.len();
    let mut dp = vec![vec![0usize; m.saturating_add(1)]; n.saturating_add(1)];
    for i in 1..=n {
        for j in 1..=m {
            if oldc[i - 1] == newc[j - 1] {
                dp[i][j] = dp[i - 1][j - 1] + 1;
            } else {
                dp[i][j] = dp[i - 1][j].max(dp[i][j - 1]);
            }
        }
    }
    let mut old_matched = vec![false; n];
    let mut new_matched = vec![false; m];
    let mut i = n;
    let mut j = m;
    while i > 0 && j > 0 {
        if oldc[i - 1] == newc[j - 1] {
            old_matched[i - 1] = true;
            new_matched[j - 1] = true;
            i -= 1;
            j -= 1;
        } else if dp[i - 1][j] >= dp[i][j - 1] {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    (old_matched, new_matched)
}

/// Appends `text` to `spans`, merging contiguous "diff" characters into a single styled span.
fn append_colored_chars(
    spans: &mut Vec<Span<'static>>,
    text: &str,
    matched: &[bool],
    base: Style,
    diff_fg: Color,
) {
    let mut buf = String::new();
    let mut prev_diff: Option<bool> = None;
    for (i, ch) in text.chars().enumerate() {
        let is_diff = !matched.get(i).copied().unwrap_or(false);
        if Some(is_diff) != prev_diff && !buf.is_empty() {
            spans.push(Span::styled(
                std::mem::take(&mut buf),
                style_for_segment(base, prev_diff, diff_fg),
            ));
        }
        prev_diff = Some(is_diff);
        buf.push(ch);
    }
    if !buf.is_empty() {
        spans.push(Span::styled(
            buf,
            style_for_segment(base, prev_diff, diff_fg),
        ));
    }
}

/// Picks the diff or base style for a contiguous segment.
fn style_for_segment(base: Style, prev_diff: Option<bool>, diff_fg: Color) -> Style {
    if prev_diff == Some(true) {
        base.fg(diff_fg)
    } else {
        base
    }
}

/// What: Renders the version cell for a package, with red/green diff highlighting when an upgrade
/// is known.
///
/// Inputs:
/// - `pkg`: Package whose version (and optional latest version) drives the rendering.
/// - `base`: Base style applied to unchanged characters and the arrow separator.
///
/// Output:
/// - A `Line` containing either the plain version or `<old> -> <new>` with diff coloring.
///
/// Details:
/// - When `pkg.latest_version` is `None`, no diff is computed and the version is returned as-is.
pub fn version_cell_line(pkg: &Package, base: Style) -> Line<'static> {
    let Some(ref latest) = pkg.latest_version else {
        return Line::from(vec![Span::styled(pkg.version.clone(), base)]);
    };
    let (old_m, new_m) = lcs_char_match_flags(&pkg.version, latest);
    let mut spans = Vec::new();
    append_colored_chars(&mut spans, &pkg.version, &old_m, base, DIFF_VERSION_RED);
    spans.push(Span::styled(" -> ", base));
    append_colored_chars(&mut spans, latest, &new_m, base, DIFF_VERSION_GREEN);
    Line::from(spans)
}
