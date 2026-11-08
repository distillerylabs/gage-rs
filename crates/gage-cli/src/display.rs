//! Text from a vault on its way to a terminal (ADR-0069). A vault is
//! untrusted, and a terminal obeys what it is sent, so a control character
//! in a title or a recipient line must arrive as text. Applied where output
//! leaves (`out`, the prompter's `say`, `prefixed`), so a new call site is
//! covered without remembering to. Secret output is the one exception, and
//! it has its own path (`secretout`).

use std::borrow::Cow;

/// `text` as one line: nothing in it can steer the terminal or start a
/// line of its own.
pub(crate) fn line(text: &str) -> Cow<'_, str> {
    escape(text, |c| c == '\n' || steers_terminal(c))
}

/// `text` keeping its line breaks, for what is legitimately several lines.
/// A field placed in such text can still forge a line, so it goes through
/// [`line`] first.
pub(crate) fn block(text: &str) -> Cow<'_, str> {
    escape(text, steers_terminal)
}

/// C0 and C1 controls and DEL, except the two that only lay text out, and
/// the bidi overrides, which make text read differently from what it says.
fn steers_terminal(c: char) -> bool {
    (c.is_control() && c != '\n' && c != '\t')
        || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// Spelled the way `{:?}` spells them, as the text gage already quotes is.
fn escape(text: &str, unsafe_char: impl Fn(char) -> bool) -> Cow<'_, str> {
    if !text.chars().any(&unsafe_char) {
        return Cow::Borrowed(text);
    }
    let mut shown = String::with_capacity(text.len());
    for c in text.chars() {
        if unsafe_char(c) {
            shown.extend(c.escape_debug());
        } else {
            shown.push(c);
        }
    }
    Cow::Owned(shown)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_escapes_everything_that_steers_a_terminal() {
        for (raw, shown) in [
            ("a\x1b[2Kb", r"a\u{1b}[2Kb"),
            ("a\rb", r"a\rb"),
            ("a\nb", r"a\nb"),
            ("a\0b", r"a\0b"),
            ("a\x7fb", r"a\u{7f}b"),
            ("a\u{9b}2Kb", r"a\u{9b}2Kb"),
            ("a\u{202e}b", r"a\u{202e}b"),
            ("a\u{2066}b", r"a\u{2066}b"),
        ] {
            assert_eq!(line(raw), shown, "{raw:?}");
        }
    }

    #[test]
    fn block_keeps_line_breaks_and_escapes_the_rest() {
        assert_eq!(block("a\x1b[1A\nb\rc\n"), "a\\u{1b}[1A\nb\\rc\n");
    }

    // A tab moves the cursor right and erases nothing, and everything
    // else here is what a title in another script looks like.
    #[test]
    fn ordinary_text_is_not_copied() {
        for text in [
            "plain",
            "a\tb",
            "caf\u{e9} \u{65e5}\u{672c}\u{8a9e}",
            "\u{26a0} warn",
        ] {
            assert!(matches!(line(text), Cow::Borrowed(_)), "{text:?}");
            assert!(matches!(block(text), Cow::Borrowed(_)), "{text:?}");
        }
    }

    // Two sinks can see the same text: a field escaped for its row, then
    // the block it lands in.
    #[test]
    fn escaping_twice_changes_nothing() {
        let once = line("a\x1b[2K\nb").into_owned();
        assert_eq!(block(&once), once);
        assert_eq!(line(&once), once);
    }
}
