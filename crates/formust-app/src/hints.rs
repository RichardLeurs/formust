//! Logic behind the editor's language-server features, kept free of window state so it can be
//! tested: dwell/debounce timing, pointer to identifier mapping, "is this code" checks for
//! completion, diagnostic marks and the squiggle geometry.

use eframe::egui::{self, Galley, Pos2, Rect, Vec2, text::CCursor};
use formust_build::Severity;
use std::ops::Range;

/// The pointer must rest this long on an identifier before hover is requested.
pub const HOVER_DELAY: f64 = 0.4;
/// Typing must pause this long before completion is requested.
pub const COMPLETE_DELAY: f64 = 0.15;

#[derive(Debug, PartialEq)]
pub enum Poll {
    /// Nothing to wait for.
    Idle,
    /// Check again in this many seconds.
    Wait(f64),
    /// The target has been stable for the delay; reported once per target.
    Ready,
}

/// Fires once after a target has stayed unchanged for a delay; a new target restarts it.
pub struct Dwell<T> {
    target: Option<T>,
    since: f64,
    fired: bool,
}

impl<T> Default for Dwell<T> {
    fn default() -> Self {
        Self {
            target: None,
            since: 0.0,
            fired: false,
        }
    }
}

impl<T: PartialEq> Dwell<T> {
    pub fn update(&mut self, target: Option<T>, now: f64, delay: f64) -> Poll {
        if self.target != target {
            self.since = now;
            self.fired = false;
            self.target = target;
        }
        if self.target.is_none() || self.fired {
            return Poll::Idle;
        }
        let left = self.since + delay - now;
        if left > 0.0 {
            Poll::Wait(left)
        } else {
            self.fired = true;
            Poll::Ready
        }
    }
}

pub fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Char range of the identifier containing the char at `idx`.
pub fn word_at(text: &str, idx: usize) -> Option<Range<usize>> {
    let chars: Vec<char> = text.chars().collect();
    if !is_ident(*chars.get(idx)?) {
        return None;
    }
    let start = chars[..idx]
        .iter()
        .rposition(|c| !is_ident(*c))
        .map_or(0, |p| p + 1);
    let end = chars[idx..]
        .iter()
        .position(|c| !is_ident(*c))
        .map_or(chars.len(), |p| idx + p);
    Some(start..end)
}

/// Char index of 0-based (`line`, `col`), clamped to the end of that line.
pub fn char_index(text: &str, line: u32, col: u32) -> usize {
    let mut offset = 0;
    for (i, l) in text.split_inclusive('\n').enumerate() {
        if i == line as usize {
            let len = l.trim_end_matches(['\n', '\r']).chars().count();
            return offset + (col as usize).min(len);
        }
        offset += l.chars().count();
    }
    offset
}

/// Whether the end of `before` is ordinary Rust code, not inside a comment (`//`, `///`, nested
/// `/* */`), a string (`"..."`, `b"..."`, raw `r#"..."#`) or a char literal (`'x'`, `'\n'`). A
/// lifetime (`'a`) is code.
pub fn in_code(before: &str) -> bool {
    #[derive(PartialEq)]
    enum S {
        Code,
        Line,
        Block(u32),
        Str,
        /// A raw string closed by `"` and this many `#`.
        Raw(usize),
        Char,
    }
    let c: Vec<char> = before.chars().collect();
    let at = |i: usize| c.get(i).copied();
    let (mut s, mut i) = (S::Code, 0);
    while i < c.len() {
        match s {
            S::Code => match (c[i], at(i + 1)) {
                ('/', Some('/')) => {
                    s = S::Line;
                    i += 1;
                }
                ('/', Some('*')) => {
                    s = S::Block(1);
                    i += 1;
                }
                ('"', _) => s = S::Str,
                // `r"`, `r#"`, `br"`: only when `r` starts a token (after an optional `b`).
                ('r', _) if raw_start(&c, i).is_some() => {
                    let (hashes, quote) = raw_start(&c, i).unwrap_or((0, i));
                    s = S::Raw(hashes);
                    i = quote;
                }
                ('\'', Some('\\')) => s = S::Char,
                // `'x'` is a char; `'a` followed by anything else is a lifetime or label.
                ('\'', Some(_)) if at(i + 2) == Some('\'') => s = S::Char,
                _ => {}
            },
            S::Line if c[i] == '\n' => s = S::Code,
            S::Block(n) => match (c[i], at(i + 1)) {
                ('*', Some('/')) => {
                    s = if n == 1 { S::Code } else { S::Block(n - 1) };
                    i += 1;
                }
                ('/', Some('*')) => {
                    s = S::Block(n + 1);
                    i += 1;
                }
                _ => {}
            },
            S::Str => match c[i] {
                '\\' => i += 1,
                '"' => s = S::Code,
                _ => {}
            },
            S::Raw(hashes) if c[i] == '"' && (1..=hashes).all(|k| at(i + k) == Some('#')) => {
                s = S::Code;
                i += hashes;
            }
            S::Char => match c[i] {
                '\\' => i += 1,
                '\'' | '\n' => s = S::Code,
                _ => {}
            },
            _ => {}
        }
        i += 1;
    }
    s == S::Code
}

/// If a raw string starts at the `r` at `i` (`r"`, `r##"`, `br"`), its number of `#` and the index of
/// its opening quote.
fn raw_start(c: &[char], i: usize) -> Option<(usize, usize)> {
    let before = i.checked_sub(1).map(|p| c[p]);
    let b_before = before == Some('b') && (i < 2 || !is_ident(c[i - 2]));
    if before.is_some_and(is_ident) && !b_before {
        return None; // the end of a longer word, such as `bar`
    }
    let hashes = c[i + 1..].iter().take_while(|&&h| h == '#').count();
    (c.get(i + 1 + hashes) == Some(&'"')).then_some((hashes, i + 1 + hashes))
}

/// Keywords after which a new name is being declared, so completing it only gets in the way.
const DECLARES: [&str; 8] = [
    "fn", "struct", "enum", "trait", "mod", "type", "union", "let",
];

/// Whether typing that left the caret at char `cursor` should pop up completion: after a `.` that
/// follows an expression (`x.`, `f().`, `v[0].`, `a?.`, not a float `1.` or a range `..`), after a
/// path `::`, or once the identifier before the caret has two characters. Never inside comments,
/// strings or char literals, after a lifetime tick, or while naming a new item or binding.
pub fn should_complete(text: &str, cursor: usize) -> bool {
    let before: String = text.chars().take(cursor).collect();
    if !in_code(&before) {
        return false;
    }
    let chars: Vec<char> = before.chars().collect();
    let word_before = |end: usize| {
        chars[..end]
            .iter()
            .rev()
            .take_while(|c| is_ident(**c))
            .count()
    };
    match chars.as_slice() {
        [.., ':', ':'] => !matches!(chars.len().checked_sub(3).map(|i| chars[i]), Some(':')),
        [.., '.', '.'] => false,
        [.., prev, '.'] => {
            let n = chars.len() - 1;
            if is_ident(*prev) {
                // A number such as `1.` or `0x1F.` is not a receiver; `x1.` is.
                let w = word_before(n);
                !chars[n - w].is_ascii_digit()
            } else {
                matches!(prev, ')' | ']' | '?' | '>')
            }
        }
        _ => {
            let word = word_before(chars.len());
            if word < 2 || chars[chars.len() - word].is_ascii_digit() {
                return false;
            }
            let start = chars.len() - word;
            if start > 0 && chars[start - 1] == '\'' {
                return false; // a lifetime or label
            }
            let head: String = chars[..start].iter().collect();
            let prev_word = head
                .trim_end()
                .rsplit(|c: char| !is_ident(c))
                .next()
                .unwrap_or_default();
            // Only when whitespace separates it from the name being typed.
            !(head.ends_with(char::is_whitespace) && DECLARES.contains(&prev_word))
        }
    }
}

/// A build diagnostic placed in a document (0-based line, column in chars).
#[derive(Clone, Debug)]
pub struct Mark {
    pub line: u32,
    pub col: u32,
    pub severity: Severity,
    pub message: String,
}

/// Marks for one file, valid only for the text the build saw.
pub struct FileMarks {
    pub path: std::path::PathBuf,
    pub text: String,
    pub items: Vec<Mark>,
}

impl Mark {
    /// Char range to underline: the identifier at the position, else the single char.
    pub fn range(&self, text: &str) -> Range<usize> {
        let at = char_index(text, self.line, self.col);
        word_at(text, at).unwrap_or(at..at + 1)
    }

    pub fn is_error(&self) -> bool {
        matches!(self.severity, Severity::Fatal | Severity::Error)
    }
}

/// What the pointer rests on: a char range and, when it is underlined, the mark.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub range: Range<usize>,
    pub mark: Option<usize>,
}

/// Screen-independent rectangle (galley space) covered by `range` on one row.
pub fn range_rect(galley: &Galley, range: &Range<usize>) -> Option<Rect> {
    let a = galley.pos_from_cursor(CCursor::new(range.start));
    let b = galley.pos_from_cursor(CCursor::new(range.end));
    (a.min.y == b.min.y && b.min.x > a.min.x).then(|| Rect::from_min_max(a.min, b.max))
}

/// The mark or identifier under `local` (a position relative to the galley's origin).
/// Marks win over plain identifiers.
pub fn target_at(galley: &Galley, text: &str, local: Vec2, marks: &[Mark]) -> Option<Target> {
    let p = local.to_pos2();
    let hit = |r: &Range<usize>| range_rect(galley, r).is_some_and(|rect| rect.contains(p));
    if let Some((i, m)) = marks.iter().enumerate().find(|(_, m)| hit(&m.range(text))) {
        return Some(Target {
            range: m.range(text),
            mark: Some(i),
        });
    }
    let idx = galley.cursor_from_pos(local).index.0;
    [Some(idx), idx.checked_sub(1)]
        .into_iter()
        .flatten()
        .filter_map(|i| word_at(text, i))
        .find(hit)
        .map(|range| Target { range, mark: None })
}

/// The zigzag along `x0..x1` at baseline `y` used for diagnostic underlines.
pub fn squiggle_points(x0: f32, x1: f32, y: f32) -> Vec<Pos2> {
    const STEP: f32 = 2.0;
    let n = ((x1 - x0) / STEP).ceil().max(1.0) as usize;
    (0..=n)
        .map(|i| {
            let x = (x0 + i as f32 * STEP).min(x1);
            Pos2::new(x, if i % 2 == 0 { y } else { y - 2.0 })
        })
        .collect()
}

/// Most lines of a hover tooltip: rust-analyzer appends the whole documentation (with examples).
const HOVER_LINES: usize = 12;

/// Keeps a hover tooltip short: the first lines, with an ellipsis when documentation was cut off.
pub fn tidy_hover(text: &str) -> String {
    let mut lines: Vec<&str> = text.trim().lines().collect();
    if lines.len() <= HOVER_LINES {
        return lines.join("\n");
    }
    lines.truncate(HOVER_LINES);
    format!("{}\n…", lines.join("\n").trim_end())
}

/// A tooltip shown near the code.
pub struct Popup {
    pub text: String,
    pub severity: Option<Severity>,
    /// Screen position of its top-left corner.
    pub pos: Pos2,
}

/// Hover bookkeeping for the active editor.
#[derive(Default)]
pub struct Hover {
    pub dwell: Dwell<(usize, usize, Option<usize>)>,
    pub target: Option<Target>,
    /// Position (0-based line, column) asked from the server and not yet answered.
    pub requested: Option<(u32, u32)>,
    pub popup: Option<Popup>,
}

impl Hover {
    pub fn clear(&mut self) {
        self.target = None;
        self.requested = None;
        self.popup = None;
    }

    /// Follows the pointer: a different target drops the popup and any pending answer.
    pub fn set_target(&mut self, target: Option<Target>) {
        if self.target != target {
            self.requested = None;
            self.popup = None;
            self.target = target;
        }
    }

    /// Accepts a server answer only if it is for the position still being asked about.
    pub fn accept(&mut self, line: u32, col: u32, text: Option<String>, pos: Pos2) {
        if self.requested != Some((line, col)) {
            return;
        }
        self.requested = None;
        self.popup = text.map(|text| Popup {
            text: tidy_hover(&text),
            severity: None,
            pos,
        });
    }
}

/// Draws a popup; it never takes the pointer, so hovering stays on the editor.
pub fn show_popup(
    ctx: &egui::Context,
    popup: &Popup,
    error: egui::Color32,
    warning: egui::Color32,
) {
    egui::Area::new(egui::Id::unique("editor-hover"))
        .order(egui::Order::Tooltip)
        .fixed_pos(popup.pos)
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(560.0);
                let mut text = egui::RichText::new(&popup.text).monospace();
                match popup.severity {
                    Some(Severity::Fatal | Severity::Error) => text = text.color(error),
                    Some(Severity::Warning) => text = text.color(warning),
                    _ => {}
                }
                ui.label(text);
            });
        });
}

/// The parts of a signature's label, in order, with the active parameter's part marked. Parameters
/// are found in the label one after another, so a repeated name is matched where it comes.
pub fn highlight_parameter(
    label: &str,
    parameters: &[String],
    active: Option<usize>,
) -> Vec<(String, bool)> {
    let mut spans: Vec<(usize, usize, bool)> = Vec::new();
    let mut from = 0;
    for (i, p) in parameters.iter().enumerate() {
        if p.is_empty() {
            continue;
        }
        if let Some(off) = label[from..].find(p.as_str()) {
            let start = from + off;
            spans.push((start, start + p.len(), Some(i) == active));
            from = start + p.len();
        }
    }
    let mut out = Vec::new();
    let mut at = 0;
    for (start, end, marked) in spans {
        if start > at {
            out.push((label[at..start].to_string(), false));
        }
        out.push((label[start..end].to_string(), marked));
        at = end;
    }
    if at < label.len() {
        out.push((label[at..].to_string(), false));
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_active_parameter_is_marked_in_the_label() {
        let params = vec!["a: Integer".to_string(), "b: Integer".to_string()];
        let got = highlight_parameter("Add(a: Integer; b: Integer):Integer", &params, Some(1));
        assert_eq!(
            got,
            vec![
                ("Add(".to_string(), false),
                ("a: Integer".to_string(), false),
                ("; ".to_string(), false),
                ("b: Integer".to_string(), true),
                ("):Integer".to_string(), false),
            ]
        );
        assert!(
            highlight_parameter("Sq(x: Integer)", &[], None)
                .iter()
                .all(|(_, m)| !m)
        );
    }

    use super::*;

    #[test]
    fn dwell_fires_once_after_the_delay_and_restarts_on_a_new_target() {
        let mut d = Dwell::default();
        assert_eq!(d.update(None::<u32>, 0.0, 0.4), Poll::Idle);
        assert!(matches!(d.update(Some(1), 1.0, 0.4), Poll::Wait(w) if (w - 0.4).abs() < 1e-9));
        assert!(matches!(d.update(Some(1), 1.3, 0.4), Poll::Wait(w) if (w - 0.1).abs() < 1e-9));
        assert_eq!(d.update(Some(1), 1.4, 0.4), Poll::Ready);
        assert_eq!(d.update(Some(1), 2.0, 0.4), Poll::Idle);
        // Moving to another target restarts the clock.
        assert!(matches!(d.update(Some(2), 2.1, 0.4), Poll::Wait(w) if (w - 0.4).abs() < 1e-9));
        assert_eq!(d.update(None, 2.2, 0.4), Poll::Idle);
        assert!(matches!(d.update(Some(2), 2.3, 0.4), Poll::Wait(w) if (w - 0.4).abs() < 1e-9));
    }

    #[test]
    fn finds_the_word_around_a_char() {
        let t = "x := foo_1 + 2;";
        assert_eq!(word_at(t, 5), Some(5..10));
        assert_eq!(word_at(t, 9), Some(5..10));
        assert_eq!(word_at(t, 10), None);
        assert_eq!(word_at(t, 0), Some(0..1));
        assert_eq!(word_at(t, 99), None);
        assert_eq!(word_at("é€ab", 2), Some(2..4)); // € is not an identifier char
    }

    #[test]
    fn maps_line_and_column_to_a_char_index() {
        let t = "ab\n😀cd\nef";
        assert_eq!(char_index(t, 0, 1), 1);
        assert_eq!(char_index(t, 1, 2), 5);
        assert_eq!(char_index(t, 1, 99), 6); // clamped to the line end
        assert_eq!(char_index(t, 2, 1), 8);
        assert_eq!(char_index(t, 9, 0), 9); // past the end
        // Round trip with the crate's own (line, col) of an index.
        for i in 0..=t.chars().count() {
            let (l, c) = crate::line_col(t, i);
            assert_eq!(char_index(t, l, c), i);
        }
    }

    #[test]
    fn tells_code_from_comments_and_strings() {
        assert!(in_code("fn main() { let x = 1; "));
        assert!(!in_code("let x = 1; // note"));
        assert!(!in_code("/// doc"));
        assert!(in_code("// note\nx"));
        assert!(!in_code("/* open"));
        assert!(in_code("/* closed */ x"));
        assert!(!in_code("/* a /* nested */ still open"));
        assert!(in_code("/* a /* nested */ closed */ x"));
        assert!(!in_code("let s = \"hel"));
        assert!(in_code("let s = \"closed\" + x"));
        assert!(
            !in_code("let s = \"say \\\"hi"),
            "an escaped quote does not close it"
        );
        assert!(in_code("let s = \"say \\\"hi\\\"\"; x"));
        assert!(!in_code("let s = \"two\nlines"), "strings span lines");
        assert!(!in_code("let s = b\"by"));
        assert!(in_code("// \"quote in a comment\nx"));
        assert!(
            in_code("let s = \"//\"; x"),
            "// in a string starts no comment"
        );
    }

    #[test]
    fn raw_strings_char_literals_and_lifetimes() {
        assert!(!in_code("let s = r\"raw "));
        assert!(
            in_code("let s = r\"raw \\\" x"),
            "no escapes in a raw string: the quote closes it"
        );
        assert!(in_code("let s = r\"raw\"; x"));
        assert!(!in_code("let s = r#\"has \" quote "));
        assert!(!in_code("let s = r##\"a \"# b "));
        assert!(in_code("let s = r##\"a \"# b\"##; x"));
        assert!(!in_code("let s = br#\"by"));
        assert!(
            in_code("let bar = 1; bar"),
            "`r` ending a word starts nothing"
        );
        assert!(in_code("let r = 1; r + 2"));
        assert!(in_code("let c = 'x'; x"));
        assert!(in_code("let c = '\"'; x"), "a quote char is not a string");
        assert!(in_code("let c = '\\''; x"));
        assert!(in_code("let c = '\\n'; x"));
        assert!(!in_code("let c = '\\u{1F600"));
        assert!(
            in_code("fn f<'a>(x: &'a str) -> &'a str { x"),
            "lifetimes are code"
        );
        assert!(in_code("'outer: loop { break 'outer; }"));
        assert!(in_code("let c = '€'; x"));
    }

    #[test]
    fn completes_after_a_dot_a_path_or_two_chars() {
        let ok = |t: &str| should_complete(t, t.chars().count());
        // Identifier prefix.
        assert!(!ok("t"));
        assert!(ok("to"));
        assert!(ok("let y = tot"));
        assert!(!ok("let y = 12"));
        assert!(!ok("let y = "));
        // Member access.
        assert!(ok("ui."));
        assert!(ok("self.name."));
        assert!(ok("items[2]."));
        assert!(ok("f(x)."));
        assert!(ok("v.iter().map(f)."));
        assert!(ok("x?."));
        assert!(ok("x1."));
        assert!(ok("Vec::<u8>::new()."));
        assert!(ok("ui.la"), "a member prefix");
        assert!(!ok("1."), "a float");
        assert!(!ok("0x1F."), "a hex number");
        assert!(!ok("for i in 0.."), "a range");
        assert!(!ok("for i in 0..="));
        assert!(!ok("x = ."));
        // Paths.
        assert!(ok("egui::"));
        assert!(ok("use std::collections::"));
        assert!(ok("Vec::<u8>::"));
        assert!(ok("<T as Trait>::"));
        assert!(!ok("x:::"));
        assert!(!ok("let a: "), "a single colon");
        // Never in comments, strings, chars.
        assert!(!ok("// to"));
        assert!(!ok("/// ui."));
        assert!(!ok("/* to"));
        assert!(!ok("let s = \"to"));
        assert!(!ok("let s = \"egui::"));
        assert!(!ok("let s = r#\"ui."));
        assert!(!ok("\"a\"."), "after a string literal");
        assert!(!ok("'a'."), "after a char literal");
        assert!(ok("// c\nui."));
        // Lifetimes and declarations.
        assert!(!ok("fn f<'st"));
        assert!(!ok("&'st"));
        assert!(!ok("fn na"));
        assert!(!ok("pub struct Lo"));
        assert!(!ok("let va"));
        assert!(ok("let mut va"));
        assert!(ok("fnord"), "`fn` only counts as a whole word");
        assert!(ok("return va"));
        // The caret decides, not the end of the text.
        let t = "ui.\nlet s = \"x";
        assert!(should_complete(t, 3));
        assert!(!should_complete(t, t.chars().count()));
    }

    #[test]
    fn marks_underline_the_identifier_or_one_char() {
        let m = |line, col| Mark {
            line,
            col,
            severity: Severity::Error,
            message: String::new(),
        };
        let t = "x := nope;\n;";
        assert_eq!(m(0, 5).range(t), 5..9);
        assert_eq!(m(0, 9).range(t), 9..10);
        assert_eq!(m(1, 0).range(t), 11..12);
    }

    #[test]
    fn squiggle_zigzags_between_two_heights_and_ends_at_the_edge() {
        let pts = squiggle_points(10.0, 17.0, 20.0);
        assert_eq!(pts.first().unwrap().x, 10.0);
        assert_eq!(pts.last().unwrap().x, 17.0);
        assert!(pts.iter().all(|p| p.y == 20.0 || p.y == 18.0));
        assert!(pts.windows(2).all(|w| w[0].y != w[1].y));
        assert_eq!(squiggle_points(5.0, 5.0, 0.0).len(), 2);
    }

    #[test]
    fn hover_text_is_cut_to_a_few_lines() {
        assert_eq!(tidy_hover("let x: u8\n\nsize = 1"), "let x: u8\n\nsize = 1");
        let long = (1..=30)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let cut = tidy_hover(&long);
        assert_eq!(cut.lines().count(), HOVER_LINES + 1);
        assert!(
            cut.starts_with("line 1\n") && cut.ends_with("line 12\n…"),
            "{cut}"
        );
    }

    #[test]
    fn hover_ignores_answers_for_an_old_position() {
        let mut h = Hover {
            requested: Some((3, 4)),
            ..Default::default()
        };
        h.accept(3, 5, Some("old".into()), Pos2::ZERO);
        assert!(h.popup.is_none());
        assert_eq!(h.requested, Some((3, 4)));
        h.accept(3, 4, Some("var x".into()), Pos2::ZERO);
        assert_eq!(h.popup.as_ref().unwrap().text, "var x");
        // After the pointer moves on, even a matching late answer is dropped.
        h.requested = Some((1, 1));
        h.set_target(Some(Target {
            range: 0..1,
            mark: None,
        }));
        h.accept(1, 1, Some("late".into()), Pos2::ZERO);
        assert!(h.popup.is_none());
    }

    /// Lays out real monospace text headlessly and finds the identifier under a point.
    #[test]
    fn maps_pointer_positions_to_identifiers_with_a_real_galley() {
        let text = "start\n  total += total + 1;\nend;";
        let ctx = egui::Context::default();
        let out = ctx.run_pass(Default::default(), |ctx| {
            let pal = crate::style::Palette::of(ctx);
            let job = crate::highlight::layout(
                text,
                &pal,
                crate::editor_font(),
                crate::prefs::line_height(),
            );
            let galley = ctx.fonts_mut(|f| f.layout_job(job));
            // Centre of char `idx` on its row.
            let centre = |idx: usize| {
                let r = galley.pos_from_cursor(CCursor::new(idx));
                Vec2::new(r.min.x + 2.0, r.center().y)
            };
            let first = text.find("total").unwrap();
            let t = target_at(&galley, text, centre(first + 2), &[]).unwrap();
            assert_eq!(t.range, first..first + 5);
            // Rows are one line height apart, so the second line is hit by its y.
            assert_eq!(
                galley.pos_from_cursor(CCursor::new(first)).min.y,
                crate::prefs::line_height()
            );
            // Whitespace between words, and the space past the end of a line, are no target.
            assert!(target_at(&galley, text, centre(first + 5), &[]).is_none());
            assert!(target_at(&galley, text, Vec2::new(900.0, 27.0), &[]).is_none());
            // A mark on `+=` wins even though it is not an identifier.
            let mark = Mark {
                line: 1,
                col: 8,
                severity: Severity::Error,
                message: "m".into(),
            };
            let at = text.find("+=").unwrap();
            let t = target_at(&galley, text, centre(at), std::slice::from_ref(&mark)).unwrap();
            assert_eq!((t.range, t.mark), (at..at + 1, Some(0)));
        });
        out.drop_without_applying_deltas();
    }
}
