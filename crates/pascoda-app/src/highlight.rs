//! Minimal Pascal syntax highlighting. To be replaced by tree-sitter-pascal.

use crate::prefs;
use crate::style::Palette;
use eframe::egui::{Color32, FontId, TextFormat, text::LayoutJob};

const KEYWORDS: &[&str] = &[
    "and",
    "array",
    "as",
    "begin",
    "case",
    "class",
    "const",
    "constructor",
    "destructor",
    "div",
    "do",
    "downto",
    "else",
    "end",
    "except",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "implementation",
    "in",
    "inherited",
    "interface",
    "is",
    "mod",
    "nil",
    "not",
    "of",
    "or",
    "private",
    "procedure",
    "program",
    "property",
    "protected",
    "public",
    "record",
    "repeat",
    "set",
    "shl",
    "shr",
    "then",
    "to",
    "true",
    "try",
    "type",
    "unit",
    "until",
    "uses",
    "var",
    "while",
    "with",
    "xor",
];

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Plain,
    Keyword,
    String,
    Comment,
    Number,
    UserType,
    BuiltinType,
    Call,
}

/// The token kinds the Preferences window sets colours for, with their labels. The order is the
/// order of `prefs::TOKEN_KEYS`.
pub const TOKENS: [(Kind, &str); prefs::TOKEN_COUNT] = [
    (Kind::Plain, "Text"),
    (Kind::Keyword, "Keywords"),
    (Kind::String, "Strings"),
    (Kind::Comment, "Comments"),
    (Kind::Number, "Numbers"),
    (Kind::UserType, "User types"),
    (Kind::BuiltinType, "Built-in types"),
    (Kind::Call, "Calls"),
];

/// The colour of a token: the user's choice for this appearance, else the palette's.
fn color(kind: Kind, pal: &Palette) -> Color32 {
    let slot = TOKENS.iter().position(|(k, _)| *k == kind).unwrap_or(0);
    match prefs::code_color(pal.dark, slot) {
        Some([r, g, b]) => Color32::from_rgb(r, g, b),
        None => default_color(kind, pal),
    }
}

/// The palette's colour of a token, before any user choice.
pub fn default_color(kind: Kind, pal: &Palette) -> Color32 {
    match kind {
        Kind::Plain => pal.code_text,
        Kind::Keyword => pal.keyword,
        Kind::String => pal.string,
        Kind::Comment => pal.comment,
        Kind::Number => pal.number,
        Kind::UserType => pal.user_type,
        Kind::BuiltinType => pal.builtin_type,
        Kind::Call => pal.call,
    }
}

const BUILTIN_TYPES: &[&str] = &[
    "integer",
    "string",
    "boolean",
    "char",
    "byte",
    "word",
    "longint",
    "int64",
    "double",
    "single",
    "real",
    "extended",
    "pointer",
    "shortint",
    "smallint",
    "cardinal",
    "longword",
    "qword",
    "widestring",
    "ansistring",
    "shortstring",
    "variant",
    "pchar",
    "tobject",
    "tclass",
];

/// `TForm1`, `TButton`: a `T` followed by a capital, the convention for Pascal types.
fn looks_like_type(word: &str) -> bool {
    let mut c = word.chars();
    c.next() == Some('T') && c.next().is_some_and(|x| x.is_ascii_uppercase())
}

/// Splits `text` into (kind, byte range) tokens covering the whole string.
fn tokenize(text: &str) -> Vec<(Kind, usize, usize)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let start = i;
        let kind = match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                Kind::Comment
            }
            b'{' => {
                while i < b.len() && b[i] != b'}' {
                    i += 1;
                }
                i = (i + 1).min(b.len());
                Kind::Comment
            }
            b'(' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b')')) {
                    i += 1;
                }
                i = (i + 2).min(b.len());
                Kind::Comment
            }
            b'\'' => {
                i += 1;
                while i < b.len() && b[i] != b'\'' && b[i] != b'\n' {
                    i += 1;
                }
                i = (i + 1).min(b.len());
                Kind::String
            }
            c if c.is_ascii_digit() => {
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'.') {
                    i += 1;
                }
                Kind::Number
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                let word = &text[start..i];
                let lower = word.to_ascii_lowercase();
                if KEYWORDS.contains(&lower.as_str()) {
                    Kind::Keyword
                } else if BUILTIN_TYPES.contains(&lower.as_str()) {
                    Kind::BuiltinType
                } else if looks_like_type(word) {
                    Kind::UserType
                } else if text[i..].trim_start_matches([' ', '\t']).starts_with('(') {
                    Kind::Call
                } else {
                    Kind::Plain
                }
            }
            _ => {
                // Advance by a whole UTF-8 character so slices stay valid.
                i += text[i..].chars().next().map_or(1, char::len_utf8);
                Kind::Plain
            }
        };
        out.push((kind, start, i));
    }
    out
}

/// The highlighter for a file, picked by its extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    Pascal,
    /// Lazarus `.lfm` and Delphi `.dfm` form text.
    Form,
    /// XML (`.xml`, `.plist`) and the XML project files of Lazarus and Delphi (`.lpi`, `.lpk`, `.dproj`).
    Markup,
    Json,
    /// `.ini`, `.toml`, `.cfg`.
    Config,
    Yaml,
    Shell,
    Markdown,
    /// Anything without colours (`.txt` and the rest).
    Text,
}

/// The language of a file, from its extension (case does not matter).
pub fn language_for(path: &std::path::Path) -> Language {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "pas" | "pp" | "lpr" | "dpr" | "dpk" | "inc" => Language::Pascal,
        "lfm" | "dfm" => Language::Form,
        "xml" | "lpi" | "lpk" | "dproj" | "plist" => Language::Markup,
        "json" => Language::Json,
        "ini" | "toml" | "cfg" => Language::Config,
        "yml" | "yaml" => Language::Yaml,
        "sh" => Language::Shell,
        "md" | "markdown" => Language::Markdown,
        _ => Language::Text,
    }
}

/// The tokens of `text` for `lang`: contiguous spans that cover the whole text.
pub fn tokenize_for(lang: Language, text: &str) -> Vec<(Kind, usize, usize)> {
    match lang {
        Language::Pascal => tokenize(text),
        Language::Form => scan(text, form_token),
        Language::Markup => scan(text, markup_token),
        Language::Json => scan(text, json_token),
        Language::Config => scan(text, config_token),
        Language::Yaml => scan(text, yaml_token),
        Language::Shell => scan(text, shell_token),
        Language::Markdown => scan(text, markdown_token),
        Language::Text => scan(text, |_, _| None),
    }
}

/// `line_height` spaces the lines out the way Xcode does (about 1.5 times the font size).
pub fn layout(text: &str, pal: &Palette, font: FontId, line_height: f32) -> LayoutJob {
    layout_for(Language::Pascal, text, pal, font, line_height)
}

/// [`layout`] for a file of language `lang`.
pub fn layout_for(lang: Language, text: &str, pal: &Palette, font: FontId, line_height: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    for (kind, s, e) in tokenize_for(lang, text) {
        let mut format = TextFormat::simple(font.clone(), color(kind, pal));
        format.line_height = Some(line_height);
        job.append(&text[s..e], 0.0, format);
    }
    job
}

/// Walks `text`: at each position `token` may return a token (its kind and length in bytes); anything else
/// is plain text, and plain runs are merged. The spans cover the whole text.
fn scan(text: &str, mut token: impl FnMut(&str, usize) -> Option<(Kind, usize)>) -> Vec<(Kind, usize, usize)> {
    let mut out: Vec<(Kind, usize, usize)> = Vec::new();
    let mut i = 0;
    while i < text.len() {
        if let Some((kind, len)) = token(text, i).filter(|(_, n)| *n > 0 && i + *n <= text.len()) {
            out.push((kind, i, i + len));
            i += len;
            continue;
        }
        let w = text[i..].chars().next().map_or(1, char::len_utf8);
        match out.last_mut() {
            Some(last) if last.0 == Kind::Plain && last.2 == i => last.2 = i + w,
            _ => out.push((Kind::Plain, i, i + w)),
        }
        i += w;
    }
    out
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Whether a word starts at `i` (not in the middle of one).
fn word_start(text: &str, i: usize) -> bool {
    i == 0 || !is_ident(text.as_bytes()[i - 1])
}

/// The length of the identifier-like word at `i` (letters, digits, `_`).
fn word_len(text: &str, i: usize) -> usize {
    text.as_bytes()[i..].iter().take_while(|b| is_ident(**b)).count()
}

/// The end of the line that holds `i` (its `\n`, or the end of the text).
fn line_end(text: &str, i: usize) -> usize {
    text[i..].find('\n').map_or(text.len(), |k| i + k)
}

fn line_start(text: &str, i: usize) -> bool {
    i == 0 || text.as_bytes()[i - 1] == b'\n'
}

/// A quoted run starting at `i` with quote `q`, up to its closing quote on the same line (or the line's end).
fn quoted(text: &str, i: usize, q: u8) -> usize {
    let bytes = text.as_bytes();
    let end = line_end(text, i);
    (i + 1..end).find(|&j| bytes[j] == q).map_or(end - i, |j| j + 1 - i)
}

/// Whether the next non-blank character after `i + len` is `sep` (a key before `=` or `:`).
fn followed_by(text: &str, i: usize, len: usize, sep: u8) -> bool {
    text.as_bytes()[i + len..].iter().find(|b| **b != b' ' && **b != b'\t').is_some_and(|b| *b == sep)
}

/// A keyword-like word at `i` (whole words only): its length, when it is one of `words`.
fn word_in(text: &str, i: usize, words: &[&str]) -> Option<usize> {
    if !word_start(text, i) {
        return None;
    }
    let n = word_len(text, i);
    let word = text[i..i + n].to_ascii_lowercase();
    (n > 0 && words.contains(&word.as_str())).then_some(n)
}

fn is_number_at(text: &str, i: usize) -> Option<usize> {
    let n = word_len(text, i);
    (n > 0 && word_start(text, i) && text.as_bytes()[i].is_ascii_digit()).then_some(n)
}

/// `object`, `inherited` and friends, strings (with `#nn` character codes), numbers, and class names.
fn form_token(text: &str, i: usize) -> Option<(Kind, usize)> {
    match text.as_bytes()[i] {
        b'\'' => Some((Kind::String, quoted(text, i, b'\''))),
        b'#' => {
            let n = text.as_bytes()[i + 1..].iter().take_while(|b| b.is_ascii_digit()).count();
            (n > 0).then_some((Kind::String, 1 + n))
        }
        _ => {
            if let Some(n) = word_in(text, i, &["object", "inherited", "inline", "end"]) {
                return Some((Kind::Keyword, n));
            }
            if let Some(n) = is_number_at(text, i) {
                return Some((Kind::Number, n));
            }
            if word_start(text, i) && text.is_char_boundary(i) {
                let n = word_len(text, i);
                if n > 0 && looks_like_type(&text[i..i + n]) {
                    return Some((Kind::UserType, n));
                }
            }
            None
        }
    }
}

/// Tags (`<name`, `</name`), attribute values and comments.
fn markup_token(text: &str, i: usize) -> Option<(Kind, usize)> {
    let rest = &text[i..];
    if rest.starts_with("<!--") {
        let end = rest.find("-->").map_or(text.len(), |k| i + k + 3);
        return Some((Kind::Comment, end - i));
    }
    let b = text.as_bytes();
    match b[i] {
        b'<' => {
            let start = if b.get(i + 1) == Some(&b'/') { i + 2 } else { i + 1 };
            let n = b[start.min(b.len())..]
                .iter()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(**c, b'-' | b'_' | b':' | b'.'))
                .count();
            (n > 0).then_some((Kind::Keyword, start + n - i))
        }
        b'"' => Some((Kind::String, quoted(text, i, b'"'))),
        _ => None,
    }
}

/// Strings and numbers, `true`/`false`/`null`, and the keys (a string or a word before `:`).
fn json_token(text: &str, i: usize) -> Option<(Kind, usize)> {
    match text.as_bytes()[i] {
        b'"' => Some((Kind::String, quoted(text, i, b'"'))),
        b'-' | b'0'..=b'9' => {
            let n = text.as_bytes()[i..]
                .iter()
                .take_while(|b| b.is_ascii_digit() || matches!(**b, b'-' | b'+' | b'.' | b'e' | b'E'))
                .count();
            (n > 0 && (text.as_bytes()[i] != b'-' || text.as_bytes().get(i + 1).is_some_and(u8::is_ascii_digit)))
                .then_some((Kind::Number, n))
        }
        _ => word_in(text, i, &["true", "false", "null"]).map(|n| (Kind::Keyword, n)),
    }
}

/// `[section]` headers, `#` and `;` comments, quoted values, numbers, booleans and keys (a word before `=`).
fn config_token(text: &str, i: usize) -> Option<(Kind, usize)> {
    match text.as_bytes()[i] {
        b'#' | b';' => Some((Kind::Comment, line_end(text, i) - i)),
        b'[' if line_start(text, i) => {
            let end = line_end(text, i);
            let close = text[i..end].find(']').map_or(end - i, |k| k + 1);
            Some((Kind::Keyword, close))
        }
        b'"' | b'\'' => Some((Kind::String, quoted(text, i, text.as_bytes()[i]))),
        _ => {
            if let Some(n) = word_in(text, i, &["true", "false", "yes", "no", "on", "off"]) {
                return Some((Kind::Keyword, n));
            }
            if let Some(n) = is_number_at(text, i) {
                return Some((Kind::Number, n));
            }
            let n = word_len(text, i);
            (n > 0 && word_start(text, i) && followed_by(text, i, n, b'=')).then_some((Kind::Call, n))
        }
    }
}

/// `#` comments, quoted strings, numbers, booleans and keys (a word before `:`).
fn yaml_token(text: &str, i: usize) -> Option<(Kind, usize)> {
    match text.as_bytes()[i] {
        b'#' => Some((Kind::Comment, line_end(text, i) - i)),
        b'"' | b'\'' => Some((Kind::String, quoted(text, i, text.as_bytes()[i]))),
        _ => {
            if let Some(n) = word_in(text, i, &["true", "false", "null", "yes", "no", "on", "off"]) {
                return Some((Kind::Keyword, n));
            }
            if let Some(n) = is_number_at(text, i) {
                return Some((Kind::Number, n));
            }
            let n = word_len(text, i);
            (n > 0 && word_start(text, i) && followed_by(text, i, n, b':')).then_some((Kind::Call, n))
        }
    }
}

/// `#` comments, quoted strings, `$variables`, and the shell's keywords.
fn shell_token(text: &str, i: usize) -> Option<(Kind, usize)> {
    match text.as_bytes()[i] {
        b'#' => Some((Kind::Comment, line_end(text, i) - i)),
        b'"' | b'\'' => Some((Kind::String, quoted(text, i, text.as_bytes()[i]))),
        b'$' => {
            let n = word_len(text, i + 1);
            (n > 0).then_some((Kind::BuiltinType, 1 + n))
        }
        _ => {
            if let Some(n) = word_in(
                text,
                i,
                &["if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac", "in", "function", "return", "exit", "export", "local"],
            ) {
                return Some((Kind::Keyword, n));
            }
            is_number_at(text, i).map(|n| (Kind::Number, n))
        }
    }
}

/// Headings (a line starting with `#`), inline code and fenced code lines.
fn markdown_token(text: &str, i: usize) -> Option<(Kind, usize)> {
    match text.as_bytes()[i] {
        b'#' if line_start(text, i) => Some((Kind::Keyword, line_end(text, i) - i)),
        b'`' if text[i..].starts_with("```") && line_start(text, i) => Some((Kind::String, line_end(text, i) - i)),
        b'`' => Some((Kind::String, quoted(text, i, b'`'))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_colour_from_the_preferences_replaces_the_palette_colour_for_its_appearance() {
        let _guard = prefs::tests::APPLY.write().unwrap_or_else(|e| e.into_inner());
        let mut p = prefs::Prefs::default();
        p.set_color(false, 1, Some([1, 2, 3]));
        p.apply();
        assert_eq!(color(Kind::Keyword, &Palette::LIGHT), Color32::from_rgb(1, 2, 3));
        assert_eq!(color(Kind::Keyword, &Palette::DARK), Palette::DARK.keyword, "the dark colour is not overridden");
        assert_eq!(color(Kind::String, &Palette::LIGHT), Palette::LIGHT.string);
        prefs::Prefs::default().apply();
        assert_eq!(color(Kind::Keyword, &Palette::LIGHT), Palette::LIGHT.keyword);
    }

    fn kinds(src: &str) -> Vec<(Kind, &str)> {
        tokenize(src)
            .into_iter()
            .map(|(k, s, e)| (k, &src[s..e]))
            .filter(|(k, _)| *k != Kind::Plain)
            .collect()
    }

    #[test]
    fn classifies_tokens() {
        let got = kinds("BEGIN WriteLn('hi'); // x\nend.");
        assert_eq!(
            got,
            vec![
                (Kind::Keyword, "BEGIN"),
                (Kind::Call, "WriteLn"),
                (Kind::String, "'hi'"),
                (Kind::Comment, "// x"),
                (Kind::Keyword, "end"),
            ]
        );
    }

    #[test]
    fn tells_types_and_calls_from_plain_names() {
        let got = kinds("var x: Integer; f: TForm1; begin Foo(x); y := z; end");
        assert!(got.contains(&(Kind::BuiltinType, "Integer")));
        assert!(got.contains(&(Kind::UserType, "TForm1")));
        assert!(got.contains(&(Kind::Call, "Foo")));
        assert!(!got.iter().any(|(_, w)| *w == "x" || *w == "y" || *w == "z"));
        assert!(!looks_like_type("Tab") && !looks_like_type("T") && looks_like_type("TX"));
    }

    fn kinds_for(lang: Language, src: &str) -> Vec<(Kind, &str)> {
        tokenize_for(lang, src)
            .into_iter()
            .map(|(k, s, e)| (k, &src[s..e]))
            .filter(|(k, _)| *k != Kind::Plain)
            .collect()
    }

    #[test]
    fn a_file_is_highlighted_by_its_extension() {
        use std::path::Path;
        assert_eq!(language_for(Path::new("unit1.pas")), Language::Pascal);
        assert_eq!(language_for(Path::new("notes.lpr")), Language::Pascal);
        assert_eq!(language_for(Path::new("MainForm.LFM")), Language::Form);
        assert_eq!(language_for(Path::new("app.lpi")), Language::Markup);
        assert_eq!(language_for(Path::new("pascoda.toml")), Language::Config);
        assert_eq!(language_for(Path::new("build.sh")), Language::Shell);
        assert_eq!(language_for(Path::new("README.md")), Language::Markdown);
        assert_eq!(language_for(Path::new("notes.txt")), Language::Text);
        assert_eq!(language_for(Path::new("LICENSE")), Language::Text);
    }

    #[test]
    fn every_language_covers_the_whole_text_with_unicode_and_unterminated_runs() {
        let samples = [
            "unit é; {\u{1F600}} 'open",
            "object Form1: TForm1\n  Caption = 'Hi é'#13#10\n  Left = 10\nend\n",
            "<?xml version=\"1.0\"?>\n<!-- c é -->\n<Project><Item a=\"1\"/></Project>",
            "{\"a\": [1, -2.5e3, true, null], \"é\": \"x\n",
            "[core]\n# note é\nname = \"x\"\nflag = yes\n",
            "key: value # c\n- 'quoted'\nnum: 12\n",
            "#!/bin/sh\nif [ \"$HOME\" ]; then echo 'é'; fi # c\n",
            "# Title é\nText `code` and\n```\nfenced\n```\n",
            "plain é text\n",
        ];
        for lang in [
            Language::Pascal,
            Language::Form,
            Language::Markup,
            Language::Json,
            Language::Config,
            Language::Yaml,
            Language::Shell,
            Language::Markdown,
            Language::Text,
        ] {
            for src in samples {
                let spans = tokenize_for(lang, src);
                let mut at = 0;
                for (_, s, e) in &spans {
                    assert_eq!(*s, at, "{lang:?}: spans must be contiguous");
                    assert!(e > s);
                    at = *e;
                }
                assert_eq!(at, src.len(), "{lang:?}: spans must cover the text");
            }
        }
    }

    #[test]
    fn each_language_colours_its_own_tokens() {
        assert_eq!(
            kinds_for(Language::Json, "{\"a\": [1, true, null]}"),
            vec![(Kind::String, "\"a\""), (Kind::Number, "1"), (Kind::Keyword, "true"), (Kind::Keyword, "null")]
        );
        assert_eq!(
            kinds_for(Language::Config, "[core]\n# note\nname = \"x\"\nflag = yes\n"),
            vec![(Kind::Keyword, "[core]"), (Kind::Comment, "# note"), (Kind::Call, "name"), (Kind::String, "\"x\""), (Kind::Call, "flag"), (Kind::Keyword, "yes")]
        );
        assert_eq!(
            kinds_for(Language::Yaml, "key: 12 # c\n"),
            vec![(Kind::Call, "key"), (Kind::Number, "12"), (Kind::Comment, "# c")]
        );
        assert_eq!(
            kinds_for(Language::Shell, "if [ \"$HOME\" ]; then echo 'x'; fi # c"),
            vec![
                (Kind::Keyword, "if"),
                (Kind::String, "\"$HOME\""),
                (Kind::Keyword, "then"),
                (Kind::String, "'x'"),
                (Kind::Keyword, "fi"),
                (Kind::Comment, "# c"),
            ]
        );
        assert_eq!(
            kinds_for(Language::Markup, "<Project a=\"1\"/><!-- c -->"),
            vec![(Kind::Keyword, "<Project"), (Kind::String, "\"1\""), (Kind::Comment, "<!-- c -->")]
        );
        assert_eq!(
            kinds_for(Language::Form, "object Form1: TForm1\n  Caption = 'Hi'#13\n  Left = 10\nend\n"),
            vec![
                (Kind::Keyword, "object"),
                (Kind::UserType, "TForm1"),
                (Kind::String, "'Hi'"),
                (Kind::String, "#13"),
                (Kind::Number, "10"),
                (Kind::Keyword, "end"),
            ]
        );
        assert_eq!(
            kinds_for(Language::Markdown, "# Title\ntext `code`\n"),
            vec![(Kind::Keyword, "# Title"), (Kind::String, "`code`")]
        );
        assert!(kinds_for(Language::Text, "if then 'x' # 1").is_empty(), "plain text has no colours");
    }

    #[test]
    fn survives_unterminated_and_unicode() {
        let _ = tokenize("{ open comment é");
        let _ = tokenize("'open string é");
        let _ = tokenize("(* open");
    }
}
