//! Free text (commons plan D1, D2, S0.1): what a line of talk or a letter may
//! hold, so the client counts what the server will keep.
//!
//! - **A line** ([`clean`]) is trimmed, its runs of whitespace folded to one
//!   space, and its control characters dropped. A newline or a tab is
//!   whitespace, so it folds too: a line is one line. At most [`MAX_LINE`]
//!   characters.
//! - **A letter** ([`clean_letter`]) keeps its newlines. Each of its lines is
//!   cleaned as a line is, and a run of blank lines folds to one, so a letter
//!   keeps its paragraphs. At most [`MAX_LETTER`] characters, newlines
//!   among them.
//! - **Characters, not bytes.** A limit counts `char`s, so a line of emoji
//!   is as long as a line of letters.
//! - **Direction overrides go** (U+202A to U+202E, U+2066 to U+2069). They
//!   aren't control characters to Rust, but they can turn the rest of a
//!   line, and the name after it, round. Joiners stay: emoji need them.
//! - **No word filter** (D2).
//!
//! The server cleans what it's sent and refuses what this refuses, so a
//! client that cleans first shows what will be said.

/// The most characters in a line of talk.
pub const MAX_LINE: usize = 240;

/// The most characters in a letter, newlines among them.
pub const MAX_LETTER: usize = 1000;

/// How far a local line carries, in world units (D1): 3 m.
pub const HEARD: f64 = 30.0;

/// How many Capitol lines a new subscriber is sent (D1).
pub const CAPITOL_BACKLOG: usize = 40;

/// Why a line or a letter was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TalkError {
    /// Nothing left once it's cleaned
    Empty,
    /// More than `max` characters once it's cleaned
    TooLong { max: usize },
}

impl TalkError {
    /// Words a client can show.
    pub fn message(&self) -> String {
        match self {
            TalkError::Empty => "There's nothing to send".into(),
            TalkError::TooLong { max } => format!("That's over {max} characters"),
        }
    }
}

impl std::fmt::Display for TalkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for TalkError {}

/// A line of talk as it's said, or why it can't be.
pub fn clean(text: &str) -> Result<String, TalkError> {
    within(fold(text), MAX_LINE)
}

/// A letter as it's sent, or why it can't be.
pub fn clean_letter(text: &str) -> Result<String, TalkError> {
    let mut letter = String::new();
    let mut blank = false;
    for line in text.lines().map(fold) {
        if line.is_empty() {
            blank = !letter.is_empty();
            continue;
        }
        if !letter.is_empty() {
            letter.push_str(if blank { "\n\n" } else { "\n" });
        }
        letter.push_str(&line);
        blank = false;
    }
    within(letter, MAX_LETTER)
}

/// One line, trimmed and folded, with the control characters gone.
fn fold(text: &str) -> String {
    let kept: String = text.chars().filter(|&c| !dropped(c)).collect();
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A character that's dropped: a control character that isn't whitespace
/// (which folds instead), or a direction override.
fn dropped(c: char) -> bool {
    (c.is_control() && !c.is_whitespace())
        || matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

fn within(text: String, max: usize) -> Result<String, TalkError> {
    if text.is_empty() {
        Err(TalkError::Empty)
    } else if text.chars().count() > max {
        Err(TalkError::TooLong { max })
    } else {
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_trimmed_and_its_whitespace_folded() {
        assert_eq!(clean("  hello   there  ").unwrap(), "hello there");
        assert_eq!(clean("tabs\t\tand\u{a0}spaces").unwrap(), "tabs and spaces");
    }

    #[test]
    fn a_line_loses_its_newlines_and_control_characters() {
        assert_eq!(clean("one\ntwo\r\nthree").unwrap(), "one two three");
        assert_eq!(clean("bell\u{7}ring\u{1b}[31m").unwrap(), "bellring[31m");
        assert_eq!(clean("null\0byte").unwrap(), "nullbyte");
        assert_eq!(
            clean("buy from \u{202E}nomis\u{202C} now").unwrap(),
            "buy from nomis now",
            "a direction override can't turn the line round"
        );
    }

    #[test]
    fn joiners_stay_so_emoji_hold_together() {
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        assert_eq!(clean(family).unwrap(), family);
    }

    #[test]
    fn nothing_left_is_refused() {
        assert_eq!(clean(""), Err(TalkError::Empty));
        assert_eq!(clean(" \n\t "), Err(TalkError::Empty));
        assert_eq!(clean("\u{7}\u{1b}"), Err(TalkError::Empty));
        assert_eq!(clean_letter("\n\n  \n"), Err(TalkError::Empty));
    }

    #[test]
    fn the_limit_counts_characters_not_bytes() {
        let emoji = "\u{1F33E}".repeat(MAX_LINE);
        assert_eq!(emoji.len(), 4 * MAX_LINE);
        assert_eq!(clean(&emoji).unwrap(), emoji, "240 emoji are 240 characters");
        let over = "\u{1F33E}".repeat(MAX_LINE + 1);
        assert_eq!(clean(&over), Err(TalkError::TooLong { max: MAX_LINE }));
        assert!(clean(&"a".repeat(MAX_LINE)).is_ok());
        assert!(clean(&"a".repeat(MAX_LINE + 1)).is_err());
    }

    #[test]
    fn the_limit_is_counted_after_cleaning() {
        let padded = format!("   {}   ", "a".repeat(MAX_LINE));
        assert!(clean(&padded).is_ok(), "the trimmed spaces don't count");
        let spaced = "a    ".repeat(MAX_LINE / 2);
        assert_eq!(clean(&spaced).unwrap().chars().count(), MAX_LINE - 1);
    }

    #[test]
    fn a_letter_keeps_its_newlines_and_paragraphs() {
        let letter = "  Dear Ada,\r\n\r\n\r\nThe   ore came.\nThanks!\u{7}  \n\n";
        assert_eq!(
            clean_letter(letter).unwrap(),
            "Dear Ada,\n\nThe ore came.\nThanks!"
        );
        assert_eq!(clean_letter("\n\nhi").unwrap(), "hi", "leading blank lines go");
    }

    #[test]
    fn a_letter_may_be_longer_than_a_line() {
        let long = "a".repeat(MAX_LETTER);
        assert_eq!(clean_letter(&long).unwrap(), long);
        assert_eq!(
            clean_letter(&"a".repeat(MAX_LETTER + 1)),
            Err(TalkError::TooLong { max: MAX_LETTER })
        );
        // Newlines count.
        let lines = "a\n".repeat(MAX_LETTER / 2 + 1);
        assert!(clean_letter(&lines).is_err());
    }

    #[test]
    fn the_errors_have_words() {
        assert_eq!(TalkError::Empty.message(), "There's nothing to send");
        assert_eq!(
            TalkError::TooLong { max: 240 }.to_string(),
            "That's over 240 characters"
        );
    }
}
