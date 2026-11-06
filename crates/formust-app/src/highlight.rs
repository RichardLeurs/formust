//! Hand-written syntax highlighting. Rust is the editor's language; the other kinds are the plain
//! text files a project holds (Cargo's `.toml`, Markdown, shell scripts and the like).

use crate::prefs;
use crate::style::Palette;
use eframe::egui::{Color32, FontId, TextFormat, text::LayoutJob};

const KEYWORDS: &[&str] = &[
    "abstract", "as", "async", "await", "become", "box", "break", "const", "continue", "crate",
    "do", "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "if", "impl", "in",
    "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref",
    "return", "Self", "self", "static", "struct", "super", "trait", "true", "try", "type",
    "typeof", "unsafe", "unsized", "use", "virtual", "where", "while", "yield",
];

/// The primitive types, and the std names every file sees (the prelude's types).
const BUILTIN_TYPES: &[&str] = &[
    "bool", "char", "str", "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64",
    "u128", "usize", "f32", "f64", "String", "Option", "Result", "Vec", "Box",
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
    /// `#[derive(Debug)]` and `#![allow(..)]`.
    Attribute,
    /// `'a` and `'static`.
    Lifetime,
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
    (Kind::Attribute, "Attributes"),
    (Kind::Lifetime, "Lifetimes"),
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
        Kind::Attribute => pal.attribute,
        Kind::Lifetime => pal.lifetime,
    }
}

/// Splits `text` into (kind, byte range) tokens covering the whole string.
fn tokenize(text: &str) -> Vec<(Kind, usize, usize)> {
    scan(text, rust_token)
}

/// The highlighter for a file, picked by its extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    /// `.rs`, which includes the form files `name.frm.rs` (their extension is `rs` too).
    Rust,
    /// XML (`.xml`, `.plist`).
    Markup,
    Json,
    /// `.ini`, `.toml`, `.cfg`: Cargo's manifests are `.toml`.
    Config,
    Yaml,
    Shell,
    Markdown,
    /// Anything without colours (`.txt` and the rest).
    Text,
}

/// The language of a file, from its extension (case does not matter).
pub fn language_for(path: &std::path::Path) -> Language {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "rs" => Language::Rust,
        "xml" | "plist" => Language::Markup,
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
        Language::Rust => tokenize(text),
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
    layout_for(Language::Rust, text, pal, font, line_height)
}

/// [`layout`] for a file of language `lang`.
pub fn layout_for(
    lang: Language,
    text: &str,
    pal: &Palette,
    font: FontId,
    line_height: f32,
) -> LayoutJob {
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
fn scan(
    text: &str,
    mut token: impl FnMut(&str, usize) -> Option<(Kind, usize)>,
) -> Vec<(Kind, usize, usize)> {
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
    text.as_bytes()[i..]
        .iter()
        .take_while(|b| is_ident(**b))
        .count()
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
    (i + 1..end)
        .find(|&j| bytes[j] == q)
        .map_or(end - i, |j| j + 1 - i)
}

/// Whether the next non-blank character after `i + len` is `sep` (a key before `=` or `:`).
fn followed_by(text: &str, i: usize, len: usize, sep: u8) -> bool {
    text.as_bytes()[i + len..]
        .iter()
        .find(|b| **b != b' ' && **b != b'\t')
        .is_some_and(|b| *b == sep)
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

/// A Rust token at `i`: comments (block comments nest), strings (plain, byte, C and raw), characters,
/// lifetimes, numbers, attributes, keywords, types, and calls (a name before `(` or `!`).
fn rust_token(text: &str, i: usize) -> Option<(Kind, usize)> {
    let b = text.as_bytes();
    let at = |k: usize| b.get(k).copied().unwrap_or(0);
    if matches!(b[i], b'b' | b'c' | b'r')
        && let Some(token) = prefixed_string(text, i)
    {
        return Some(token);
    }
    match b[i] {
        b'/' if at(i + 1) == b'/' => Some((Kind::Comment, line_end(text, i) - i)),
        b'/' if at(i + 1) == b'*' => Some((Kind::Comment, block_comment_len(text, i))),
        b'"' => Some((Kind::String, string_len(text, i))),
        b'\'' => char_or_lifetime(text, i),
        b'#' if at(i + 1) == b'[' || (at(i + 1) == b'!' && at(i + 2) == b'[') => {
            Some((Kind::Attribute, attribute_len(text, i)))
        }
        b'0'..=b'9' => Some((Kind::Number, number_len(text, i))),
        c if is_ident(c) => {
            let n = word_len(text, i);
            let word = &text[i..i + n];
            let is_call = matches!(at(i + n), b'(' | b'!') && at(i + n + 1) != b'=';
            let kind = if KEYWORDS.contains(&word) {
                Kind::Keyword
            } else if BUILTIN_TYPES.contains(&word) {
                Kind::BuiltinType
            } else if word.starts_with(|c: char| c.is_ascii_uppercase()) {
                Kind::UserType
            } else if is_call {
                Kind::Call
            } else {
                Kind::Plain
            };
            (kind != Kind::Plain).then_some((kind, n))
        }
        _ => None,
    }
}

/// `/* ... */`, with nesting, from `i` to the matching `*/` (or the end of the text).
fn block_comment_len(text: &str, i: usize) -> usize {
    let b = text.as_bytes();
    let mut depth = 0usize;
    let mut j = i;
    while j < b.len() {
        if b[j] == b'/' && b.get(j + 1) == Some(&b'*') {
            depth += 1;
            j += 2;
        } else if b[j] == b'*' && b.get(j + 1) == Some(&b'/') {
            depth -= 1;
            j += 2;
            if depth == 0 {
                return j - i;
            }
        } else {
            j += 1;
        }
    }
    b.len() - i
}

/// A double-quoted string from the quote at `i` to its closing quote (strings may run over lines).
fn string_len(text: &str, i: usize) -> usize {
    let b = text.as_bytes();
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'\\' => j += 2,
            b'"' => return j + 1 - i,
            _ => j += 1,
        }
    }
    b.len() - i
}

/// `b"..."`, `c"..."`, `r"..."`, `r#"..."#`, `br#"..."#` and `b'x'`, from `i`; `None` for anything else
/// (including `r#ident` and words that merely start with `b`, `c` or `r`).
fn prefixed_string(text: &str, i: usize) -> Option<(Kind, usize)> {
    let b = text.as_bytes();
    let mut j = i;
    if matches!(b[j], b'b' | b'c') {
        j += 1;
    }
    if b.get(j) == Some(&b'r') {
        let hashes = b[j + 1..].iter().take_while(|c| **c == b'#').count();
        let quote = j + 1 + hashes;
        if b.get(quote) != Some(&b'"') {
            return None;
        }
        let close = format!("\"{}", "#".repeat(hashes));
        let end = text[quote + 1..]
            .find(&close)
            .map_or(text.len(), |k| quote + 1 + k + close.len());
        return Some((Kind::String, end - i));
    }
    match b.get(j) {
        Some(b'"') if j > i => Some((Kind::String, string_len(text, j) + (j - i))),
        Some(b'\'') if j > i => char_literal_len(text, j).map(|n| (Kind::String, n + (j - i))),
        _ => None,
    }
}

/// A character literal starting at the quote `i`: `'a'`, `'\n'`, `'\u{1F600}'`. `None` when it is not one.
fn char_literal_len(text: &str, i: usize) -> Option<usize> {
    let b = text.as_bytes();
    if b.get(i + 1) == Some(&b'\\') {
        // The escaped character is skipped, so `'\''` closes on its own second quote.
        let close = text.get(i + 3..)?.find('\'')?;
        return Some(4 + close);
    }
    let width = text[i + 1..].chars().next()?.len_utf8();
    (b.get(i + 1 + width) == Some(&b'\'')).then_some(width + 2)
}

/// A character literal or a lifetime at the quote `i`. A lifetime is the quote and a name (`'a`, `'static`).
fn char_or_lifetime(text: &str, i: usize) -> Option<(Kind, usize)> {
    if let Some(n) = char_literal_len(text, i) {
        return Some((Kind::String, n));
    }
    let first = text.as_bytes().get(i + 1).copied().unwrap_or(0);
    let n = word_len(text, i + 1);
    (n > 0 && (first.is_ascii_alphabetic() || first == b'_')).then_some((Kind::Lifetime, 1 + n))
}

/// A number from `i`: decimal with `_`, a fraction (not a `..` range or a method call), an exponent,
/// a `0x`, `0o` or `0b` prefix, and a type suffix such as `u8` or `f64`.
fn number_len(text: &str, i: usize) -> usize {
    let b = text.as_bytes();
    let at = |k: usize| b.get(k).copied().unwrap_or(0);
    let digits = |mut j: usize| {
        while at(j).is_ascii_digit() || at(j) == b'_' {
            j += 1;
        }
        j
    };
    if at(i) == b'0' && matches!(at(i + 1), b'x' | b'o' | b'b') {
        let mut j = i + 2;
        while is_ident(at(j)) {
            j += 1;
        }
        return j - i;
    }
    let mut j = digits(i);
    if at(j) == b'.' && at(j + 1).is_ascii_digit() {
        j = digits(j + 1);
    }
    if matches!(at(j), b'e' | b'E') {
        let sign = usize::from(matches!(at(j + 1), b'+' | b'-'));
        if at(j + 1 + sign).is_ascii_digit() {
            j = digits(j + 1 + sign);
        }
    }
    while is_ident(at(j)) {
        j += 1;
    }
    j - i
}

/// `#[...]` or `#![...]` from the `#` at `i`, to the bracket that closes it (or the end of its line).
fn attribute_len(text: &str, i: usize) -> usize {
    let b = text.as_bytes();
    let end = line_end(text, i);
    let mut depth = 0usize;
    for (k, c) in b.iter().enumerate().take(end).skip(i) {
        match c {
            b'[' => depth += 1,
            b']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return k + 1 - i;
                }
            }
            _ => {}
        }
    }
    end - i
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
            let start = if b.get(i + 1) == Some(&b'/') {
                i + 2
            } else {
                i + 1
            };
            let n = b[start.min(b.len())..]
                .iter()
                .take_while(|c| {
                    c.is_ascii_alphanumeric() || matches!(**c, b'-' | b'_' | b':' | b'.')
                })
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
                .take_while(|b| {
                    b.is_ascii_digit() || matches!(**b, b'-' | b'+' | b'.' | b'e' | b'E')
                })
                .count();
            (n > 0
                && (text.as_bytes()[i] != b'-'
                    || text.as_bytes().get(i + 1).is_some_and(u8::is_ascii_digit)))
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
            (n > 0 && word_start(text, i) && followed_by(text, i, n, b'='))
                .then_some((Kind::Call, n))
        }
    }
}

/// `#` comments, quoted strings, numbers, booleans and keys (a word before `:`).
fn yaml_token(text: &str, i: usize) -> Option<(Kind, usize)> {
    match text.as_bytes()[i] {
        b'#' => Some((Kind::Comment, line_end(text, i) - i)),
        b'"' | b'\'' => Some((Kind::String, quoted(text, i, text.as_bytes()[i]))),
        _ => {
            if let Some(n) = word_in(
                text,
                i,
                &["true", "false", "null", "yes", "no", "on", "off"],
            ) {
                return Some((Kind::Keyword, n));
            }
            if let Some(n) = is_number_at(text, i) {
                return Some((Kind::Number, n));
            }
            let n = word_len(text, i);
            (n > 0 && word_start(text, i) && followed_by(text, i, n, b':'))
                .then_some((Kind::Call, n))
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
                &[
                    "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done",
                    "case", "esac", "in", "function", "return", "exit", "export", "local",
                ],
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
        b'`' if text[i..].starts_with("```") && line_start(text, i) => {
            Some((Kind::String, line_end(text, i) - i))
        }
        b'`' => Some((Kind::String, quoted(text, i, b'`'))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_colour_from_the_preferences_replaces_the_palette_colour_for_its_appearance() {
        let _guard = prefs::tests::APPLY
            .write()
            .unwrap_or_else(|e| e.into_inner());
        let mut p = prefs::Prefs::default();
        p.set_color(false, 1, Some([1, 2, 3]));
        p.apply();
        assert_eq!(
            color(Kind::Keyword, &Palette::LIGHT),
            Color32::from_rgb(1, 2, 3)
        );
        assert_eq!(
            color(Kind::Keyword, &Palette::DARK),
            Palette::DARK.keyword,
            "the dark colour is not overridden"
        );
        assert_eq!(color(Kind::String, &Palette::LIGHT), Palette::LIGHT.string);
        prefs::Prefs::default().apply();
        assert_eq!(
            color(Kind::Keyword, &Palette::LIGHT),
            Palette::LIGHT.keyword
        );
    }

    fn kinds(src: &str) -> Vec<(Kind, &str)> {
        tokenize(src)
            .into_iter()
            .map(|(k, s, e)| (k, &src[s..e]))
            .filter(|(k, _)| *k != Kind::Plain)
            .collect()
    }

    #[test]
    fn classifies_rust_tokens() {
        let got = kinds("fn main() { let s = \"hi\"; // x\n}");
        assert_eq!(
            got,
            vec![
                (Kind::Keyword, "fn"),
                (Kind::Call, "main"),
                (Kind::Keyword, "let"),
                (Kind::String, "\"hi\""),
                (Kind::Comment, "// x"),
            ]
        );
    }

    #[test]
    fn tells_types_calls_and_macros_from_plain_names() {
        let got =
            kinds("let v: Vec<u32> = Vec::new(); println!(\"{}\", Login::ui(x)); self.total = 1;");
        assert!(got.contains(&(Kind::BuiltinType, "Vec")));
        assert!(got.contains(&(Kind::BuiltinType, "u32")));
        assert!(got.contains(&(Kind::UserType, "Login")), "{got:?}");
        assert!(got.contains(&(Kind::Call, "println")), "a macro is a call");
        assert!(got.contains(&(Kind::Keyword, "self")));
        assert!(
            !got.iter()
                .any(|(_, w)| *w == "x" || *w == "total" || *w == "s")
        );
    }

    #[test]
    fn nested_block_comments_end_at_the_matching_close() {
        let got = kinds("/* a /* b */ still */ let x = 1; /* open");
        assert_eq!(got[0], (Kind::Comment, "/* a /* b */ still */"));
        assert_eq!(got.last().unwrap(), &(Kind::Comment, "/* open"));
        assert!(got.contains(&(Kind::Keyword, "let")));
    }

    #[test]
    fn strings_raw_strings_and_escapes() {
        let src = r##"let a = "say \"hi\""; let b = r#"raw "quoted""#; let c = b"bytes"; let d = br"x";"##;
        let got = kinds(src);
        assert!(
            got.contains(&(Kind::String, "\"say \\\"hi\\\"\"")),
            "{got:?}"
        );
        assert!(
            got.contains(&(Kind::String, "r#\"raw \"quoted\"\"#")),
            "{got:?}"
        );
        assert!(got.contains(&(Kind::String, "b\"bytes\"")), "{got:?}");
        assert!(got.contains(&(Kind::String, "br\"x\"")), "{got:?}");
        // A raw string may run over lines; an identifier that starts with r or b is not a string.
        let multi = kinds("let s = r\"one\ntwo\"; let bar = 1;");
        assert!(
            multi.contains(&(Kind::String, "r\"one\ntwo\"")),
            "{multi:?}"
        );
        assert!(
            !multi
                .iter()
                .any(|(_, w)| w.starts_with("bar") && w.len() > 3),
            "{multi:?}"
        );
    }

    #[test]
    fn characters_and_lifetimes_are_told_apart() {
        let got = kinds(
            "fn f<'a>(x: &'a str, c: char) -> char { if c == 'x' || c == '\\'' { '\\u{1F600}' } else { b'\\n' as char } }",
        );
        assert!(got.contains(&(Kind::Lifetime, "'a")), "{got:?}");
        assert!(got.contains(&(Kind::String, "'x'")), "{got:?}");
        assert!(got.contains(&(Kind::String, "'\\''")), "{got:?}");
        assert!(got.contains(&(Kind::String, "'\\u{1F600}'")), "{got:?}");
        let statics = kinds("static NAME: &'static str = \"n\";");
        assert!(
            statics.contains(&(Kind::Lifetime, "'static")),
            "{statics:?}"
        );
    }

    #[test]
    fn numbers_keep_ranges_and_method_calls_apart() {
        let got = kinds("let a = 0xFF_u8 + 1_000 + 2.5e-3 + 3f64; for i in 0..10 { 1.max(2) }");
        for n in ["0xFF_u8", "1_000", "2.5e-3", "3f64", "0", "10", "1", "2"] {
            assert!(got.contains(&(Kind::Number, n)), "{n} in {got:?}");
        }
        assert!(
            got.contains(&(Kind::Call, "max")),
            "a method call is a call: {got:?}"
        );
    }

    #[test]
    fn attributes_are_their_own_colour() {
        let got = kinds("#[derive(Debug, Default)]\n#![allow(dead_code)]\nstruct A;");
        assert_eq!(got[0], (Kind::Attribute, "#[derive(Debug, Default)]"));
        assert_eq!(got[1], (Kind::Attribute, "#![allow(dead_code)]"));
        assert!(got.contains(&(Kind::Keyword, "struct")));
        assert!(got.contains(&(Kind::UserType, "A")));
    }

    #[test]
    fn a_form_file_is_highlighted_as_rust() {
        use std::path::Path;
        assert_eq!(language_for(Path::new("login.frm.rs")), Language::Rust);
        assert_eq!(language_for(Path::new("main.RS")), Language::Rust);
        assert_eq!(language_for(Path::new("Cargo.toml")), Language::Config);
    }

    #[test]
    fn a_file_is_highlighted_by_its_extension() {
        use std::path::Path;
        assert_eq!(language_for(Path::new("main.rs")), Language::Rust);
        assert_eq!(language_for(Path::new("formust.toml")), Language::Config);
        assert_eq!(language_for(Path::new("build.sh")), Language::Shell);
        assert_eq!(language_for(Path::new("README.md")), Language::Markdown);
        assert_eq!(language_for(Path::new("notes.txt")), Language::Text);
        assert_eq!(language_for(Path::new("LICENSE")), Language::Text);
    }

    fn kinds_for(lang: Language, src: &str) -> Vec<(Kind, &str)> {
        tokenize_for(lang, src)
            .into_iter()
            .map(|(k, s, e)| (k, &src[s..e]))
            .filter(|(k, _)| *k != Kind::Plain)
            .collect()
    }

    #[test]
    fn every_language_covers_the_whole_text_with_unicode_and_unterminated_runs() {
        let samples = [
            "fn é() { /* \u{1F600} 'open",
            "let s = \"Hi é\"; r#\"open raw",
            "<?xml version=\"1.0\"?>\n<!-- c é -->\n<Project><Item a=\"1\"/></Project>",
            "{\"a\": [1, -2.5e3, true, null], \"é\": \"x\n",
            "[core]\n# note é\nname = \"x\"\nflag = yes\n",
            "key: value # c\n- 'quoted'\nnum: 12\n",
            "#!/bin/sh\nif [ \"$HOME\" ]; then echo 'é'; fi # c\n",
            "# Title é\nText `code` and\n```\nfenced\n```\n",
            "plain é text\n",
            "#[derive(\u{e9}",
            "'\\",
            "b'",
        ];
        for lang in [
            Language::Rust,
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
            vec![
                (Kind::String, "\"a\""),
                (Kind::Number, "1"),
                (Kind::Keyword, "true"),
                (Kind::Keyword, "null")
            ]
        );
        assert_eq!(
            kinds_for(
                Language::Config,
                "[core]\n# note\nname = \"x\"\nflag = yes\n"
            ),
            vec![
                (Kind::Keyword, "[core]"),
                (Kind::Comment, "# note"),
                (Kind::Call, "name"),
                (Kind::String, "\"x\""),
                (Kind::Call, "flag"),
                (Kind::Keyword, "yes")
            ]
        );
        assert_eq!(
            kinds_for(Language::Yaml, "key: 12 # c\n"),
            vec![
                (Kind::Call, "key"),
                (Kind::Number, "12"),
                (Kind::Comment, "# c")
            ]
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
            vec![
                (Kind::Keyword, "<Project"),
                (Kind::String, "\"1\""),
                (Kind::Comment, "<!-- c -->")
            ]
        );
        assert_eq!(
            kinds_for(Language::Markdown, "# Title\ntext `code`\n"),
            vec![(Kind::Keyword, "# Title"), (Kind::String, "`code`")]
        );
        assert!(
            kinds_for(Language::Text, "if then 'x' # 1").is_empty(),
            "plain text has no colours"
        );
    }

    #[test]
    fn survives_unterminated_and_unicode() {
        let _ = tokenize("/* open comment é");
        let _ = tokenize("\"open string é");
        let _ = tokenize("r#\"open");
        let _ = tokenize("'\\u{");
    }
}
