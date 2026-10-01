//! Default colours of the host terminal, for answering OSC 10 / 11 / 12 queries.
//!
//! wtmux draws every pane cell with the default colour, so the colours a pane
//! really shows by default are the host terminal's. A child that asks for
//! them (`OSC 11 ; ?`, which Neovim and others use to tell a dark theme from a
//! light one) gets the answer wtmux learned from the host at startup
//! (`ui::host_probe`). Nothing is invented: without a host answer the query
//! stays unanswered, as before.
//!
//! Measured on WezTerm, Windows Terminal Preview 1.25 and the Ghostty Windows
//! port: every one of them answers `ESC ] N ; rgb:RRRR/GGGG/BBBB` (16 bits per
//! channel, never `rgba:`), in query order and before a CPR sent after the
//! queries; WezTerm and Windows Terminal end the reply with ST whatever the
//! query ended with, Ghostty mirrors the query's terminator.

use std::sync::OnceLock;

/// A colour with 16 bits per channel, as xterm reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb16 {
    pub r: u16,
    pub g: u16,
    pub b: u16,
}

/// What the host reported for OSC 10 (foreground), 11 (background) and 12
/// (cursor). A field is `None` when the host did not answer that query.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostColors {
    pub fg: Option<Rgb16>,
    pub bg: Option<Rgb16>,
    pub cursor: Option<Rgb16>,
}

impl HostColors {
    /// The colour for OSC `osc` (10, 11 or 12).
    pub fn get(&self, osc: u8) -> Option<Rgb16> {
        match osc {
            10 => self.fg,
            11 => self.bg,
            12 => self.cursor,
            _ => None,
        }
    }
}

static HOST_COLORS: OnceLock<HostColors> = OnceLock::new();

/// Remember what the host reported (the first call wins).
pub fn set_host_colors(colors: HostColors) {
    let _ = HOST_COLORS.set(colors);
}

/// What the host reported, or nothing known if it never did.
pub fn host_colors() -> HostColors {
    HOST_COLORS.get().copied().unwrap_or_default()
}

/// Parse an X colour spec: `rgb:R/G/B` with 1 to 4 hex digits per channel
/// (scaled to 16 bits as xterm does) or `#rrggbb`. Anything else, including
/// `rgba:`, is rejected.
pub fn parse_color(spec: &str) -> Option<Rgb16> {
    if let Some(rest) = spec.strip_prefix("rgb:") {
        let mut parts = rest.split('/');
        let r = channel(parts.next()?)?;
        let g = channel(parts.next()?)?;
        let b = channel(parts.next()?)?;
        if parts.next().is_some() {
            return None;
        }
        return Some(Rgb16 { r, g, b });
    }
    if let Some(hex) = spec.strip_prefix('#') {
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| u16::from_str_radix(&hex[i..i + 2], 16).ok().map(|v| v * 257);
        return Some(Rgb16 { r: byte(0)?, g: byte(2)?, b: byte(4)? });
    }
    None
}

/// One `rgb:` channel: 1 to 4 hex digits, scaled so that all-ones is 0xffff.
fn channel(s: &str) -> Option<u16> {
    if s.is_empty() || s.len() > 4 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(s, 16).ok()?;
    let max = (1u32 << (4 * s.len())) - 1;
    Some(((value * 65535 + max / 2) / max) as u16)
}

/// Longest colour spec taken from a host reply.
const MAX_SPEC_LEN: usize = 64;

/// Pull the OSC 10 / 11 / 12 answers out of what the host sent back (other
/// bytes, such as a trailing CPR, are skipped). BEL, ESC `\` and C1 ST end a
/// reply.
pub fn parse_host_replies(text: &str) -> HostColors {
    let mut colors = HostColors::default();
    let mut rest = text;
    while let Some(start) = rest.find("\x1b]") {
        rest = &rest[start + 2..];
        let Some(semi) = rest.find(';') else { break };
        let Ok(osc) = rest[..semi].parse::<u8>() else { continue };
        let body = &rest[semi + 1..];
        let end = body
            .find(|c| c == '\x07' || c == '\x1b' || c == '\u{9c}')
            .unwrap_or(body.len());
        let spec = &body[..end];
        if spec.len() <= MAX_SPEC_LEN {
            if let Some(color) = parse_color(spec) {
                match osc {
                    10 => colors.fg = Some(color),
                    11 => colors.bg = Some(color),
                    12 => colors.cursor = Some(color),
                    _ => {}
                }
            }
        }
        rest = &body[end..];
    }
    colors
}

/// The answer to an `OSC osc ; ?` query, ended with BEL when the query was.
/// Built only from the validated channels, never from host text.
pub fn format_reply(osc: u8, color: Rgb16, bel: bool) -> Vec<u8> {
    let end = if bel { "\x07" } else { "\x1b\\" };
    format!(
        "\x1b]{};rgb:{:04x}/{:04x}/{:04x}{}",
        osc, color.r, color.g, color.b, end
    )
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(r: u16, g: u16, b: u16) -> Rgb16 {
        Rgb16 { r, g, b }
    }

    #[test]
    fn rgb_channels_scale_to_sixteen_bits() {
        assert_eq!(parse_color("rgb:ffff/0000/8080"), Some(rgb(0xffff, 0, 0x8080)));
        assert_eq!(parse_color("rgb:ff/00/80"), Some(rgb(0xffff, 0, 0x8080)));
        assert_eq!(parse_color("rgb:f/0/8"), Some(rgb(0xffff, 0, 0x8888)));
        assert_eq!(parse_color("rgb:fff/000/800"), Some(rgb(0xffff, 0, 0x8008)));
        assert_eq!(parse_color("rgb:B2B2/b2b2/B2b2"), Some(rgb(0xb2b2, 0xb2b2, 0xb2b2)));
    }

    #[test]
    fn hash_form_expands_each_byte() {
        assert_eq!(parse_color("#ff0080"), Some(rgb(0xffff, 0, 0x8080)));
        assert_eq!(parse_color("#0C0c0C"), Some(rgb(0x0c0c, 0x0c0c, 0x0c0c)));
    }

    #[test]
    fn malformed_specs_are_rejected() {
        for bad in [
            "", "rgb:", "rgb:ffff/ffff", "rgb:ffff/ffff/ffff/ffff", "rgb:/ffff/ffff",
            "rgb:fffff/0/0", "rgb:gg/00/00", "rgb:ff/00/0x", "rgba:ffff/ffff/ffff/ffff",
            "#fff", "#ff00ff00", "#gg0000", "ffffff", "rgb:ff/00/00;", "rgb: ff/00/00",
        ] {
            assert_eq!(parse_color(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn host_replies_are_found_among_other_bytes() {
        // What WezTerm, Windows Terminal and Ghostty sent back in the
        // 2026-10-01 measurements: OSC 10, OSC 11, then the CPR.
        let wez = parse_host_replies(
            "\x1b]10;rgb:b2b2/b2b2/b2b2\x1b\\\x1b]11;rgb:0000/0000/0000\x1b\\\x1b[1;1R",
        );
        assert_eq!(wez.fg, Some(rgb(0xb2b2, 0xb2b2, 0xb2b2)));
        assert_eq!(wez.bg, Some(rgb(0, 0, 0)));
        assert_eq!(wez.cursor, None);

        // A BEL-ended reply, the cursor colour, and noise around them.
        let bel = parse_host_replies("junk\x1b]12;rgb:ffff/ffff/ffff\x07\x1b[1;1R\x1b]11;rgb:0c0c/0c0c/0c0c\x07");
        assert_eq!(bel.cursor, Some(rgb(0xffff, 0xffff, 0xffff)));
        assert_eq!(bel.bg, Some(rgb(0x0c0c, 0x0c0c, 0x0c0c)));
        assert_eq!(bel.fg, None);
    }

    #[test]
    fn host_replies_ignore_other_codes_and_garbage() {
        assert_eq!(parse_host_replies(""), HostColors::default());
        assert_eq!(parse_host_replies("\x1b[1;1R"), HostColors::default());
        // OSC 4 (palette) is not one of ours; a broken body is skipped.
        assert_eq!(
            parse_host_replies("\x1b]4;0;rgb:1d1d/1f1f/2121\x1b\\\x1b]11;nonsense\x1b\\"),
            HostColors::default()
        );
        // An over-long spec is dropped, a later good reply still counts.
        let long = format!("\x1b]10;rgb:{}/0/0\x1b\\\x1b]11;#000000\x1b\\", "f".repeat(80));
        let got = parse_host_replies(&long);
        assert_eq!(got.fg, None);
        assert_eq!(got.bg, Some(rgb(0, 0, 0)));
        // No terminator at all: the text up to the end is the spec.
        assert_eq!(parse_host_replies("\x1b]11;#ffffff").bg, Some(rgb(0xffff, 0xffff, 0xffff)));
    }

    #[test]
    fn replies_are_formatted_with_the_requested_terminator() {
        let c = rgb(0xb2b2, 0x0001, 0xffff);
        assert_eq!(format_reply(10, c, false), b"\x1b]10;rgb:b2b2/0001/ffff\x1b\\".to_vec());
        assert_eq!(format_reply(11, c, true), b"\x1b]11;rgb:b2b2/0001/ffff\x07".to_vec());
    }
}
