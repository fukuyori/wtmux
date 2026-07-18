//! Terminal display-width helpers shared by the screen model and UI.

use unicode_width::UnicodeWidthChar;

/// Display width for a character, with Nerd Font / Powerline PUA handling.
#[inline]
pub(crate) fn char_width(ch: char) -> usize {
    let cp = ch as u32;
    if (0xE000..=0xF8FF).contains(&cp)
        || (0xF0000..=0xFFFFF).contains(&cp)
        || (0x100000..=0x10FFFF).contains(&cp)
    {
        return 1;
    }
    ch.width().unwrap_or(1)
}

#[inline]
pub(crate) fn str_display_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

/// Keep the longest prefix that fits in `max_width` terminal cells.
pub(crate) fn truncate_to_display_width(s: &str, max_width: usize) -> String {
    let mut out = String::new();
    let mut width = 0;
    for ch in s.chars() {
        let ch_width = char_width(ch);
        if width + ch_width > max_width {
            break;
        }
        out.push(ch);
        width += ch_width;
    }
    out
}

/// Keep the longest suffix that fits in `max_width` terminal cells.
///
/// This is used for editable text where the cursor is at the end. The result
/// is built from `char` boundaries, so multi-byte UTF-8 input is never sliced
/// at an invalid byte offset.
pub(crate) fn truncate_tail_to_display_width(s: &str, max_width: usize) -> String {
    let mut reversed = Vec::new();
    let mut width = 0;

    for ch in s.chars().rev() {
        let ch_width = char_width(ch);
        if width + ch_width > max_width {
            break;
        }
        reversed.push(ch);
        width += ch_width;
    }

    reversed.reverse();
    while reversed.first().is_some_and(|ch| char_width(*ch) == 0) {
        reversed.remove(0);
    }
    reversed.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_counts_ascii_cjk_and_private_use_cells() {
        assert_eq!(char_width('a'), 1);
        assert_eq!(char_width('日'), 2);
        assert_eq!(char_width('\u{e0b0}'), 1);
        assert_eq!(str_display_width("abc日本語"), 9);
    }

    #[test]
    fn prefix_truncation_respects_cell_width() {
        assert_eq!(truncate_to_display_width("abc日本語", 5), "abc日");
        assert_eq!(truncate_to_display_width("日本語abc", 4), "日本");
        assert_eq!(truncate_to_display_width("abc", 2), "ab");
    }

    #[test]
    fn tail_truncation_preserves_utf8_boundaries() {
        assert_eq!(truncate_tail_to_display_width("abc日本語", 5), "本語");
        assert_eq!(truncate_tail_to_display_width("日本語abc", 5), "語abc");
        assert_eq!(truncate_tail_to_display_width("日本語", 3), "語");
    }
}
