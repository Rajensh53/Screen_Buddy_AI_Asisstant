//! The prompt sent to the model, and cleaning of the selected text before it goes in.

use crate::provider::ChatRequest;

/// The prompt from the project brief, word for word.
pub const SYSTEM_PROMPT: &str = concat!(
    "You are a friendly pocket explainer for a Pakistani user.\n",
    "Detect the input language yourself. Keep answers short.\n",
    "If the input is a single word or short phrase:\n",
    "  Meaning (simple English): <1 line>\n",
    "  Urdu: <meaning in Urdu script>\n",
    "  Type: <noun/verb/adjective/...>\n",
    "  Example: <1 short sentence in the input's language>\n",
    "If the input is a sentence:\n",
    "  Simple English: <translation or plain rewording>\n",
    "  Urdu: <translation in Urdu script>\n",
    "  Note: <only if there is an idiom or slang; otherwise leave this line out>\n",
    "Roman Urdu is Urdu written in English letters; treat it as Urdu.\n",
    "No extra text.",
);

/// Same minimum as the trigger rules: shorter text is not worth explaining.
const MIN_CHARS: usize = 2;

/// Invisible characters that apps leave in selections:
/// - U+FFFC object replacement (Chromium/Electron put it where an icon is)
/// - U+200B zero-width space, U+FEFF byte order mark, U+00AD soft hyphen
///
/// U+200C/U+200D (zero-width non-joiner/joiner) are kept: Urdu script uses them.
const INVISIBLE: &[char] = &['\u{FFFC}', '\u{200B}', '\u{FEFF}', '\u{00AD}'];

/// Removes invisible characters, normalizes line endings, trims line ends and collapses
/// runs of blank lines into one. `None` if fewer than 2 characters are left.
pub fn clean_input(text: &str) -> Option<String> {
    let text: String = text.chars().filter(|c| !INVISIBLE.contains(c)).collect();
    let text = text.replace("\r\n", "\n").replace('\r', "\n");

    let mut out = String::with_capacity(text.len());
    let mut after_blank = false;
    for line in text.split('\n').map(str::trim_end) {
        if line.is_empty() {
            after_blank = true;
            continue;
        }
        if !out.is_empty() {
            out.push_str(if after_blank { "\n\n" } else { "\n" });
        }
        after_blank = false;
        out.push_str(line);
    }

    let out = out.trim_start();
    (out.chars().count() >= MIN_CHARS).then(|| out.to_owned())
}

/// The request for one explanation. `text` should come from `clean_input`.
pub fn build_request<'a>(model: &'a str, text: &'a str) -> ChatRequest<'a> {
    ChatRequest { model, system: SYSTEM_PROMPT, user: text }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(text: &str) -> Option<String> {
        clean_input(text)
    }

    #[test]
    fn object_replacement_from_antigravity_is_removed() {
        assert_eq!(clean("\u{FFFC}\nCLAUDE.md\n:").as_deref(), Some("CLAUDE.md\n:"));
        assert_eq!(clean("\u{FFFC}word").as_deref(), Some("word"));
    }

    #[test]
    fn other_invisible_characters_are_removed() {
        assert_eq!(clean("\u{FEFF}zero\u{200B}width soft\u{00AD}hyphen").as_deref(), Some("zerowidth softhyphen"));
    }

    #[test]
    fn urdu_joiners_and_text_are_kept() {
        let urdu = "خوش\u{200C}قسمتی سے یہ ملا";
        assert_eq!(clean(urdu).as_deref(), Some(urdu));
        assert_eq!(clean("mujhe neend aa rahi hai").as_deref(), Some("mujhe neend aa rahi hai"));
    }

    #[test]
    fn line_endings_are_normalized_and_line_ends_trimmed() {
        assert_eq!(clean("one  \r\ntwo\rthree\t").as_deref(), Some("one\ntwo\nthree"));
    }

    #[test]
    fn blank_line_runs_collapse_to_one() {
        assert_eq!(clean("\n\nfirst\n\n\n  \nsecond\n\n").as_deref(), Some("first\n\nsecond"));
    }

    #[test]
    fn indentation_inside_is_kept_but_leading_space_trimmed() {
        assert_eq!(clean("  fn main() {\n    run();\n}").as_deref(), Some("fn main() {\n    run();\n}"));
    }

    #[test]
    fn too_short_or_only_invisible_is_none() {
        assert_eq!(clean("\u{FFFC}\n\u{200B} \n"), None);
        assert_eq!(clean("\u{FFFC}a"), None);
        assert_eq!(clean("ab").as_deref(), Some("ab"));
    }

    #[test]
    fn request_uses_the_system_prompt_and_cleaned_text() {
        let req = build_request("qwen3.5:4b", "serendipity");
        assert_eq!(req.model, "qwen3.5:4b");
        assert_eq!(req.system, SYSTEM_PROMPT);
        assert_eq!(req.user, "serendipity");
    }

    #[test]
    fn system_prompt_matches_the_brief() {
        for line in [
            "  Meaning (simple English): <1 line>\n",
            "  Urdu: <meaning in Urdu script>\n",
            "  Type: <noun/verb/adjective/...>\n",
            "  Example: <1 short sentence in the input's language>\n",
            "  Note: <only if there is an idiom or slang; otherwise leave this line out>\n",
            "Roman Urdu is Urdu written in English letters; treat it as Urdu.\n",
        ] {
            assert!(SYSTEM_PROMPT.contains(line), "missing: {line}");
        }
        assert!(SYSTEM_PROMPT.ends_with("No extra text."));
    }
}
