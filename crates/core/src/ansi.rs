//! Terminal escape removal for captured backend output.

use std::borrow::Cow;

const ESC: char = '\u{1b}';
const BEL: char = '\u{7}';

/// Removes CSI (`ESC [ … final`) and OSC (`ESC ] … BEL | ESC \`) sequences.
/// A sequence cut off by the end of the line is dropped through the end; an
/// ESC starting neither kind is dropped alone. Lines without ESC are returned
/// borrowed.
pub fn strip_ansi(line: &str) -> Cow<'_, str> {
    if !line.contains(ESC) {
        return Cow::Borrowed(line);
    }
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c != ESC {
            out.push(c);
            continue;
        }
        match chars.peek() {
            Some('[') => {
                chars.next();
                while chars
                    .next_if(|c| ('\u{30}'..='\u{3f}').contains(c))
                    .is_some()
                {}
                while chars
                    .next_if(|c| ('\u{20}'..='\u{2f}').contains(c))
                    .is_some()
                {}
                chars.next_if(|c| ('\u{40}'..='\u{7e}').contains(c));
            }
            Some(']') => {
                chars.next();
                while let Some(c) = chars.next_if(|&c| c != ESC) {
                    if c == BEL {
                        break;
                    }
                }
                if chars.peek() == Some(&ESC) {
                    let mut rest = chars.clone();
                    rest.next();
                    if rest.peek() == Some(&'\\') {
                        chars = rest;
                        chars.next();
                    }
                }
            }
            _ => {}
        }
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_ansi_removes_singbox_color_codes() {
        assert_eq!(
            strip_ansi("\u{1b}[31mFATAL\u{1b}[0m[0000] start service: boom"),
            "FATAL[0000] start service: boom"
        );
        assert_eq!(strip_ansi("\u{1b}[3\u{80}x"), "\u{80}x");
    }

    #[test]
    fn strip_ansi_plain_line_is_borrowed() {
        let line = "2026/09/14 09:57:30.532567 [Info] plain";
        assert!(matches!(strip_ansi(line), Cow::Borrowed(s) if s == line));
    }

    #[test]
    fn strip_ansi_truncated_csi_at_end_drops_tail() {
        assert_eq!(strip_ansi("WARN\u{1b}[3"), "WARN");
        assert_eq!(strip_ansi("x\u{1b}["), "x");
    }

    #[test]
    fn strip_ansi_removes_osc_terminated_by_bel_and_st() {
        assert_eq!(strip_ansi("\u{1b}]0;title\u{7}plain out"), "plain out");
        assert_eq!(strip_ansi("a\u{1b}]8;;http://x\u{1b}\\b"), "ab");
    }

    #[test]
    fn strip_ansi_unterminated_osc_drops_tail() {
        assert_eq!(strip_ansi("keep\u{1b}]0;never ends"), "keep");
    }

    #[test]
    fn strip_ansi_drops_lone_escape() {
        assert_eq!(strip_ansi("a\u{1b}Xb"), "aXb");
        assert_eq!(strip_ansi("end\u{1b}"), "end");
    }
}
