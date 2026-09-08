//! Making remote-authored text safe to print.

/// `text` with terminal control characters escaped.
///
/// A manifest's message and its paths are written by whoever published the
/// package, and stdout is usually an interpreting terminal: an escape sequence
/// in either would move the cursor, repaint the screen, or — with OSC 52 —
/// write the reader's clipboard, all from a `quilt pull`. Escaping makes the
/// bytes visible instead of letting the terminal act on them.
///
/// Newlines and tabs pass through, being the only control characters a commit
/// message legitimately carries and harmless to a terminal.
pub fn printable(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() && c != '\n' && c != '\t' {
            // `escape_unicode` renders `\u{1b}`, which names the byte rather
            // than hiding it behind a placeholder.
            out.extend(c.escape_unicode());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::printable;

    #[test]
    fn ordinary_text_is_untouched() {
        assert_eq!(printable("r3: adds add/six.txt"), "r3: adds add/six.txt");
    }

    /// The one that matters: a published message carrying a CSI sequence must
    /// reach the terminal as characters, not as an instruction.
    #[test]
    fn an_escape_sequence_is_shown_rather_than_run() {
        let hostile = "clean\u{1b}[2J\u{1b}[Hnothing here";
        let safe = printable(hostile);
        assert!(
            !safe.contains('\u{1b}'),
            "an ESC reached the terminal: {safe:?}"
        );
        assert_eq!(safe, "clean\\u{1b}[2J\\u{1b}[Hnothing here");
    }

    /// C1 carries its own control introducer, so escaping C0 alone would leave
    /// the same attack one byte away.
    #[test]
    fn the_c1_introducer_is_escaped_too() {
        assert_eq!(printable("a\u{9b}31m"), "a\\u{9b}31m");
    }

    #[test]
    fn a_multi_line_message_keeps_its_lines() {
        assert_eq!(printable("first\nsecond\tthird"), "first\nsecond\tthird");
    }
}
