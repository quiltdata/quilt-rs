//! Making remote-authored text safe to print.

/// `text` with terminal control characters escaped, keeping line structure.
///
/// A manifest's message and its logical keys are written by whoever published
/// the package, and stdout is usually an interpreting terminal: an escape
/// sequence in either would move the cursor, repaint the screen, or — with
/// OSC 52 — write the reader's clipboard, all from a `quilt pull`. Escaping
/// makes the bytes visible instead of letting the terminal act on them.
///
/// Newlines and tabs pass through, which is right for prose and **wrong for a
/// value printed inside a list** — use [`printable_line`] there.
pub fn printable(text: &str) -> String {
    escaped(text, |c| c == '\n' || c == '\t')
}

/// `text` with every control character escaped, including newline and tab.
///
/// For a value the layout gives one line of its own — a path under a group
/// heading. A newline kept there does not merely look wrong: the reader cannot
/// tell a forged line from a real one, so a published path containing
/// `"\n3 files removed:\n  important.parquet"` would invent a group and an
/// entry in the report. Prose keeps its lines; a list item cannot be allowed
/// to make new ones.
pub fn printable_line(text: &str) -> String {
    escaped(text, |_| false)
}

fn escaped(text: &str, keep: impl Fn(char) -> bool) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() && !keep(c) {
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
    use super::printable_line;

    #[test]
    fn ordinary_text_is_untouched() {
        assert_eq!(printable("r3: adds add/six.txt"), "r3: adds add/six.txt");
        assert_eq!(printable_line("add/six.txt"), "add/six.txt");
    }

    /// The one that matters for a message: a published one carrying a CSI
    /// sequence must reach the terminal as characters, not as an instruction.
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

    /// The one that matters for a path: the report puts one path per line, so a
    /// path allowed to carry a newline can forge a group heading and an entry
    /// under it — a report claiming a file was deleted that was not.
    #[test]
    fn a_path_cannot_forge_a_line() {
        let forged = "legit.csv\n3 files removed:\n  important.parquet";
        let safe = printable_line(forged);
        assert!(!safe.contains('\n'), "a path kept a line break: {safe:?}");
        assert_eq!(
            safe,
            "legit.csv\\u{a}3 files removed:\\u{a}  important.parquet"
        );
    }

    #[test]
    fn a_path_cannot_forge_a_column_either() {
        // Tabs are the same problem one character along: the report's own
        // indentation is whitespace, and a path may not add to it.
        assert_eq!(printable_line("a\tb"), "a\\u{9}b");
    }
}
