//! QR code generation for terminal output.
//!
//! Session links are long and hard to type by hand, so sshx can optionally
//! render them as a QR code that can be scanned with a phone.

use ansi_term::Color;
use qrcode::render::unicode::Dense1x2;
use qrcode::QrCode;

/// Render an ANSI-styled QR code for a URL, prefixing every line with `prefix`.
///
/// Uses Unicode half-block characters (`Dense1x2`) with explicit black-on-white
/// coloring, so the code stays scannable on both dark and light terminal color
/// schemes.
///
/// Returns `None` if the input cannot be encoded as a QR code.
pub fn render_qr_with_prefix(url: &str, prefix: &str) -> Option<String> {
    let code = QrCode::new(url.as_bytes()).ok()?;
    let image = code
        .render::<Dense1x2>()
        .dark_color(Dense1x2::Light)
        .light_color(Dense1x2::Dark)
        .quiet_zone(true)
        .build();

    let styled = image
        .lines()
        .map(|line| format!("{prefix}{}", Color::White.on(Color::Black).paint(line)))
        .collect::<Vec<_>>()
        .join("\n");

    Some(styled)
}

/// Render an ANSI QR code for a URL, indented to line up with other output.
pub fn render_qr(url: &str) -> Option<String> {
    render_qr_with_prefix(url, "  ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Strips ANSI escape sequences from a rendered QR line.
    fn strip_ansi(line: &str) -> String {
        let mut out = String::new();
        let mut chars = line.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                // Skip the CSI introducer and its parameters, up to the final
                // byte of the sequence.
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn render_standard_url() {
        let url = "https://sshx.io/s/abc1234567#key1234567890";
        let output = render_qr(url).expect("failed to render QR code");
        assert!(!output.is_empty());

        for line in output.lines() {
            assert!(line.starts_with("  "));
            assert!(line.contains("\x1b[40;37m"));
        }
    }

    #[test]
    fn render_with_custom_prefix() {
        let output = render_qr_with_prefix("https://sshx.io/s/test#123", ">>> ")
            .expect("failed to render QR code");
        for line in output.lines() {
            assert!(line.starts_with(">>> "));
        }
    }

    #[test]
    fn render_empty_string() {
        // The empty string is still encodable as a minimal QR code.
        assert!(render_qr("").is_some());
    }

    #[test]
    fn render_read_and_write_urls() {
        let read_url = "https://sshx.io/s/abcdefghij#0123456789abcd";
        let write_url = "https://sshx.io/s/abcdefghij#0123456789abcd,9876543210zyxw";

        let qr_read = render_qr(read_url).expect("failed to render read-only QR code");
        let qr_write = render_qr(write_url).expect("failed to render writable QR code");

        // QR codes must be roughly square and fit comfortably in a terminal.
        let lines: Vec<_> = qr_read.lines().collect();
        let width = strip_ansi(lines[0]).chars().count();
        assert!(lines
            .iter()
            .all(|line| strip_ansi(line).chars().count() == width));
        assert!(
            lines.len() <= 22,
            "QR code height too large: {}",
            lines.len()
        );
        assert!(
            qr_write.lines().count() <= 22,
            "QR code height too large: {}",
            qr_write.lines().count()
        );
    }

    #[test]
    fn render_matches_encoded_matrix() {
        // Decode the rendered glyphs back into modules and check that they match
        // the QR code produced by the encoder. This catches inverted colors or
        // transposed rows, either of which would make the code unscannable.
        let url = "https://sshx.io/s/abc1234567#key1234567890";
        let code = QrCode::new(url.as_bytes()).unwrap();
        let output = render_qr(url).unwrap();

        let lines: Vec<String> = output
            .lines()
            .map(|line| strip_ansi(line)[2..].into())
            .collect();
        let quiet = 4;
        let size = code.width() + 2 * quiet;
        assert_eq!(lines.len(), size.div_ceil(2));

        let colors = code.to_colors();
        for y in 0..size {
            for x in 0..size {
                let module_is_dark = x >= quiet
                    && y >= quiet
                    && x - quiet < code.width()
                    && y - quiet < code.width()
                    && colors[(y - quiet) * code.width() + x - quiet] == qrcode::types::Color::Dark;

                // `Dense1x2` renders two module rows per terminal row; the
                // first bit is the upper module. Inverted colors mean that a
                // dark module becomes a light (empty) half-block cell.
                let symbol = match lines[y / 2].chars().nth(x).unwrap() {
                    ' ' => 0b00,
                    '▄' => 0b01,
                    '▀' => 0b10,
                    '█' => 0b11,
                    c => panic!("unexpected glyph {c:?}"),
                };
                let bit = if y % 2 == 0 { symbol >> 1 } else { symbol & 1 };
                assert_eq!(bit == 0, module_is_dark, "mismatch at ({x}, {y})");
            }
        }
    }
}
