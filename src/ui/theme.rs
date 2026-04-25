//! Shared color palette and small style helpers used across `UniPack` widgets.

use ratatui::style::{Color, Style};
use ratatui::text::Span;

/// Palette used across every widget in `UniPack`.
pub struct AppColors {
    /// Background color for surfaces and selected text.
    pub bg: Color,
    /// Default foreground for body text.
    pub fg: Color,
    /// Primary accent (titles, selection, key hints).
    pub primary: Color,
    /// Secondary accent (subtle hints, distro label).
    pub secondary: Color,
    /// Positive accent (installed status, success).
    pub accent: Color,
    /// Warning accent (outdated, search banner).
    pub warning: Color,
    /// Error accent (no PMs detected, version-diff red).
    pub error: Color,
    /// Background for inset surfaces (info strip, gauges).
    pub surface: Color,
    /// Border color for bordered blocks.
    pub border: Color,
}

impl AppColors {
    /// Construct the default palette.
    const fn new() -> Self {
        Self {
            bg: Color::Rgb(26, 27, 38),
            fg: Color::Rgb(169, 177, 214),
            primary: Color::Rgb(122, 162, 247),
            secondary: Color::Rgb(187, 154, 247),
            accent: Color::Rgb(158, 206, 106),
            warning: Color::Rgb(224, 175, 104),
            error: Color::Rgb(247, 118, 142),
            surface: Color::Rgb(36, 40, 59),
            border: Color::Rgb(65, 72, 104),
        }
    }
}

/// The single shared palette instance used by every renderer.
pub const COLORS: AppColors = AppColors::new();

/// Span styled as a footer key label (primary accent).
#[inline]
pub fn footer_key(label: &str) -> Span<'_> {
    Span::styled(label, Style::default().fg(COLORS.primary))
}

/// Span styled as a footer hint (muted secondary accent).
#[inline]
pub fn footer_hint(text: &str) -> Span<'_> {
    Span::styled(text, Style::default().fg(COLORS.secondary))
}
