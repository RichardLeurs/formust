//! The Lazarus internal tools that the Tools menu offers beside the custom tools: the text logic of Make
//! Resource String. UI-free and tested; the menu and the document update are in `main.rs`.

use std::ops::Range;

/// The string literal (`'...'`, with `''` for a quote) on the line of character `index` that contains
/// `index`. Returns its character range and its source text (quotes included).
pub fn literal_at(text: &str, index: usize) -> Option<(Range<usize>, String)> {
    let chars: Vec<char> = text.chars().collect();
    let index = index.min(chars.len());
    // The line that holds the caret.
    let start = chars[..index].iter().rposition(|&c| c == '\n').map_or(0, |p| p + 1);
    let end = chars[index..].iter().position(|&c| c == '\n').map_or(chars.len(), |p| index + p);
    let mut i = start;
    while i < end {
        if chars[i] == '\'' {
            // Scan to the closing quote, where a doubled quote is part of the text.
            let mut j = i + 1;
            loop {
                if j >= end {
                    break; // unterminated: no literal to take
                }
                if chars[j] == '\'' {
                    if j + 1 < end && chars[j + 1] == '\'' {
                        j += 2;
                        continue;
                    }
                    let range = i..j + 1;
                    if range.contains(&index) || index == range.end {
                        return Some((range.clone(), chars[range].iter().collect()));
                    }
                    break;
                }
                j += 1;
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    None
}

/// A resource name for a literal's text: `rs` and the words of the text, capitalised (`'Hello, world'`
/// gives `rsHelloWorld`). Taken names (in `taken`, compared without case) get a number.
pub fn resource_name(literal: &str, taken: &str) -> String {
    let inner = literal.trim_matches('\'').replace("''", "'");
    let mut name = String::from("rs");
    for word in inner.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
        let mut cs = word.chars();
        if let Some(first) = cs.next() {
            name.extend(first.to_uppercase());
            name.push_str(&cs.as_str().to_lowercase());
        }
    }
    if name == "rs" {
        name.push_str("Text");
    }
    let lower = taken.to_lowercase();
    let base = name.clone();
    let mut n = 2;
    while lower.contains(&name.to_lowercase()) {
        name = format!("{base}{n}");
        n += 1;
    }
    name
}

/// `text` with the string literal at `caret` replaced by a new resource name, and the declaration
/// `name = 'text';` added: to a `resourcestring` section just above `implementation` (or, in a program,
/// just above the main `begin`), creating the section when there is none. Errors name what is missing.
pub fn make_resource_string(text: &str, caret: usize) -> Result<String, String> {
    let (range, literal) = literal_at(text, caret)
        .ok_or_else(|| "Put the caret inside a string literal, such as 'Hello', first".to_string())?;
    let chars: Vec<char> = text.chars().collect();
    let name = resource_name(&literal, text);
    // The start of every line, and the line that starts with `word` (compared without case).
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(chars.iter().enumerate().filter(|(_, c)| **c == '\n').map(|(i, _)| i + 1))
        .collect();
    let line_is = |s: usize, word: &str| {
        let line: String = chars[s..].iter().take_while(|c| **c != '\n').collect();
        line.trim().eq_ignore_ascii_case(word)
    };
    let at = line_starts
        .iter()
        .copied()
        .find(|&s| line_is(s, "implementation"))
        .or_else(|| line_starts.iter().copied().find(|&s| line_is(s, "begin")))
        .ok_or_else(|| "This file has no implementation or begin to put the resource string before".to_string())?;
    // The nearest non-blank line above that point: when it is the resourcestring keyword, or a declaration
    // in one, the new declaration goes right after it, so the section grows; otherwise a new section opens.
    let above = line_starts
        .iter()
        .copied()
        .filter(|&s| s < at)
        .rev()
        .find(|&s| {
            let line: String = chars[s..].iter().take_while(|c| **c != '\n').collect();
            !line.trim().is_empty()
        });
    let in_section = above.is_some_and(|s| {
        let line: String = chars[s..].iter().take_while(|c| **c != '\n').collect();
        let t = line.trim_end();
        line_is(s, "resourcestring")
            || (line.starts_with("  ") && t.contains(" = ") && t.ends_with(';'))
    });
    let declaration = format!("  {name} = {literal};\n");
    let (insert_at, block) = if in_section {
        // The start of the line after the section's last declaration.
        let last = above.unwrap_or(0);
        let next = line_starts.iter().copied().find(|&s| s > last).unwrap_or(chars.len());
        (next, declaration)
    } else {
        (at, format!("resourcestring\n{declaration}\n"))
    };
    // Two edits in document order: the literal becomes the name, and the declaration goes in at `at`.
    let mut edits = vec![(range.start, range.end, name), (insert_at, insert_at, block)];
    edits.sort_by_key(|e| e.0);
    let mut result = String::with_capacity(text.len() + 64);
    let mut pos = 0;
    for (start, end, replacement) in edits {
        result.extend(&chars[pos..start]);
        result.push_str(&replacement);
        pos = end;
    }
    result.extend(&chars[pos..]);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_literal_the_caret_is_in() {
        let t = "x := 'Hello, world';\ny := 'it''s';";
        let caret = t.find("world").unwrap();
        assert_eq!(literal_at(t, caret).unwrap().1, "'Hello, world'");
        let second = t.find("it''s").unwrap() + 2;
        assert_eq!(literal_at(t, second).unwrap().1, "'it''s'", "a doubled quote stays inside");
        assert_eq!(literal_at(t, 0), None, "no literal on that part of the line");
    }

    #[test]
    fn names_from_the_words_and_avoids_taken_names() {
        assert_eq!(resource_name("'Hello, world'", ""), "rsHelloWorld");
        assert_eq!(resource_name("'!!'", ""), "rsText");
        assert_eq!(resource_name("'Save'", "rsSave here"), "rsSave2");
    }

    #[test]
    fn a_unit_gets_a_resourcestring_section_before_implementation() {
        let src = "unit u;\n\ninterface\n\nimplementation\n\nprocedure P;\nbegin\n  ShowMessage('Hello, world');\nend;\n\nend.\n";
        let caret = src.find("world").unwrap();
        let out = make_resource_string(src, caret).unwrap();
        assert!(out.contains("resourcestring\n  rsHelloWorld = 'Hello, world';\n\nimplementation"), "{out}");
        assert!(out.contains("ShowMessage(rsHelloWorld);"), "{out}");
        // A second literal joins the section that is now there, instead of opening another.
        let again = out.replace("ShowMessage(rsHelloWorld);", "ShowMessage('Bye now');");
        let second = make_resource_string(&again, again.find("Bye now").unwrap()).unwrap();
        assert_eq!(second.matches("resourcestring").count(), 1, "{second}");
        assert!(second.contains("  rsByeNow = 'Bye now';\n\nimplementation"), "{second}");
    }

    #[test]
    fn a_program_gets_the_section_before_begin_and_errors_are_named() {
        let src = "program p;\nbegin\n  WriteLn('Bye');\nend.\n";
        let out = make_resource_string(src, src.find("Bye").unwrap()).unwrap();
        assert!(out.contains("resourcestring\n  rsBye = 'Bye';\n\nbegin"), "{out}");
        assert!(make_resource_string(src, 0).unwrap_err().contains("string literal"));
        assert!(make_resource_string("unit u;\nbegin 'x' end.", 14).is_err(), "no line of its own holds begin");
    }
}
