//! Shared scroll math for table-like views with a centered cursor.

/// Outcome of [`compute_scroll`]: how many rows are visible and the offset of the first row.
pub struct ScrollWindow {
    /// Number of rows that fit inside the body (excluding header/border).
    pub visible_rows: usize,
    /// Index of the first visible row in the underlying list.
    pub offset: usize,
}

/// What: Computes the scroll window for a centered cursor inside an `area_height`-tall body.
///
/// Inputs:
/// - `area_height`: Full ratatui area height including border lines.
/// - `total_rows`: Total number of rows in the data source.
/// - `cursor`: Currently selected row index.
///
/// Output:
/// - A [`ScrollWindow`] describing the visible row count and start offset.
///
/// Details:
/// - Subtracts 2 from `area_height` to account for the bordered block.
/// - The cursor is centered when possible and clamped to the maximum scrollable offset.
pub fn compute_scroll(area_height: u16, total_rows: usize, cursor: usize) -> ScrollWindow {
    let visible_rows = (area_height as usize).saturating_sub(2);
    let max_scroll = total_rows.saturating_sub(visible_rows);
    let half_visible = visible_rows / 2;
    let offset = cursor.saturating_sub(half_visible).min(max_scroll);
    ScrollWindow {
        visible_rows,
        offset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_list_has_zero_offset() {
        let w = compute_scroll(10, 3, 1);
        assert_eq!(w.offset, 0);
        assert_eq!(w.visible_rows, 8);
    }

    #[test]
    fn cursor_centers_within_window() {
        let w = compute_scroll(10, 100, 50);
        assert_eq!(w.visible_rows, 8);
        assert_eq!(w.offset, 46);
    }

    #[test]
    fn offset_clamped_to_max_scroll() {
        let w = compute_scroll(10, 12, 11);
        assert_eq!(w.visible_rows, 8);
        assert_eq!(w.offset, 4);
    }
}
