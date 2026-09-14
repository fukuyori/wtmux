//! Terminal display-width helpers shared by the screen model and UI.

use std::sync::atomic::{AtomicBool, Ordering};

use unicode_width::UnicodeWidthChar;

use super::emoji_text_default::TEXT_DEFAULT_EMOJI;

/// Whether the host terminal draws a text-default emoji followed by U+FE0F
/// (❤️, ✔️, ⚠️ ...) in two cells. Measured once at startup by
/// `ui::host_probe`; `false` (one cell, unicode-width's answer) until then.
static VS16_EMOJI_WIDE: AtomicBool = AtomicBool::new(false);

pub(crate) fn set_vs16_emoji_wide(wide: bool) {
    VS16_EMOJI_WIDE.store(wide, Ordering::Relaxed);
}

pub(crate) fn vs16_emoji_wide() -> bool {
    VS16_EMOJI_WIDE.load(Ordering::Relaxed)
}

/// Emoji=Yes, Emoji_Presentation=No: one cell on its own, two with U+FE0F on
/// terminals that honour emoji presentation.
pub(crate) fn is_text_default_emoji(ch: char) -> bool {
    let cp = ch as u32;
    TEXT_DEFAULT_EMOJI
        .binary_search_by(|&(lo, hi)| {
            if cp < lo {
                std::cmp::Ordering::Greater
            } else if cp > hi {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// Display width for a character, with Nerd Font / Powerline PUA handling.
///
/// Deviations from `unicode-width` are limited to cases where every terminal
/// wtmux was measured against (inbox conhost, Windows Terminal, WezTerm)
/// agrees on a different answer; see `docs/design-width-model.md`.
#[inline]
pub(crate) fn char_width(ch: char) -> usize {
    let cp = ch as u32;
    if (0xE000..=0xF8FF).contains(&cp)
        || (0xF0000..=0xFFFFF).contains(&cp)
        || (0x100000..=0x10FFFF).contains(&cp)
    {
        return 1;
    }
    // Halfwidth dakuten / handakuten (ｶﾞ ﾊﾟ) and the halfwidth Hangul filler
    // are spacing characters (East_Asian_Width=H) that every measured terminal
    // draws in their own cell; unicode-width 0.1 reports them as zero-width.
    if (0xFF9E..=0xFFA0).contains(&cp) {
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
    fn halfwidth_dakuten_and_hangul_filler_are_one_cell() {
        // East_Asian_Width=H spacing marks: every measured terminal draws
        // them in their own cell (unicode-width 0.1 says 0).
        assert_eq!(char_width('\u{FF9E}'), 1);
        assert_eq!(char_width('\u{FF9F}'), 1);
        assert_eq!(char_width('\u{FFA0}'), 1);
        assert_eq!(str_display_width("\u{FF76}\u{FF9E}"), 2);
        // Ordinary combining marks are still zero-width.
        assert_eq!(char_width('\u{0300}'), 0);
        assert_eq!(char_width('\u{3099}'), 0);
    }

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
