//! Class completion (Ctrl+Shift+C in Lazarus), on the text of a unit. A method declared in a class of the
//! interface section but missing from the implementation gets an empty body at the end of the
//! implementation; a method implemented as `TClass.Method` but not declared in its class gets a declaration
//! at the end of the class. UI-free and tested; the editor update is in `main.rs`.

/// The result: the new text, the number of method bodies added and of declarations added.
#[derive(Debug, PartialEq, Eq)]
pub struct Completed {
    pub text: String,
    pub bodies: usize,
    pub declarations: usize,
}

struct Method {
    keyword: String,
    name: String,
    /// The signature as written after the name: `(X: Integer)`, `: Integer`, or empty.
    sig: String,
}

struct Class {
    name: String,
    indent: String,
    /// The line of the class's `end;`.
    end_line: usize,
    methods: Vec<Method>,
}

fn leading(line: &str) -> String {
    line.chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}

/// The class a line declares (`TFoo = class`, or `TFoo = class(TBar)`), not a forward declaration.
fn class_start(line: &str) -> Option<String> {
    let t = line.trim();
    let (name, rest) = t.split_once('=')?;
    let name = name.trim();
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    let rest = rest.trim();
    let is_class = rest.len() >= 5 && rest[..5].eq_ignore_ascii_case("class") && rest[5..].trim_start().chars().next().is_none_or(|c| c == '(' || c == ';' || c.is_whitespace());
    // A forward declaration (`TFoo = class;`) has no members.
    (is_class && !rest[5..].trim_start().starts_with(';')).then(|| name.to_string())
}

/// The keyword, name, and signature of a method header (`procedure Bar(X: Integer);`), up to the `;`
/// that ends the header (directives such as `virtual;` are not part of the signature).
fn header(line: &str) -> Option<Method> {
    let t = line.trim_start();
    let keyword = ["procedure", "function", "constructor", "destructor"]
        .into_iter()
        .find(|k| t.len() > k.len() && t[..k.len()].eq_ignore_ascii_case(k) && t[k.len()..].starts_with(' '))?;
    let rest = t[keyword.len()..].trim_start();
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.').collect();
    if name.is_empty() {
        return None;
    }
    let after = &rest[name.len()..];
    // The signature runs to the first `;` outside parentheses.
    let mut depth = 0;
    let mut end = after.len();
    for (i, c) in after.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ';' if depth == 0 => {
                end = i;
                break;
            }
            _ => {}
        }
    }
    Some(Method { keyword: keyword.to_string(), name, sig: after[..end].trim().to_string() })
}

/// Completes the classes of a unit. Errors when the text has no interface or implementation section.
pub fn complete(text: &str) -> Result<Completed, String> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let iface = lines
        .iter()
        .position(|l| l.trim().eq_ignore_ascii_case("interface"))
        .ok_or("This is not a unit with an interface section")?;
    let imp = lines
        .iter()
        .position(|l| l.trim().eq_ignore_ascii_case("implementation"))
        .ok_or("This unit has no implementation section")?;

    // The classes of the interface, with their methods.
    let mut classes: Vec<Class> = Vec::new();
    let mut i = iface;
    while i < imp {
        if let Some(name) = class_start(lines[i]) {
            let indent = leading(lines[i]);
            let mut j = i + 1;
            let mut methods = Vec::new();
            loop {
                if j >= imp {
                    return Err(format!("class {name} has no end"));
                }
                let t = lines[j].trim();
                if leading(lines[j]) == indent && (t.eq_ignore_ascii_case("end;") || t.eq_ignore_ascii_case("end")) {
                    break;
                }
                if let Some(m) = header(lines[j]) {
                    methods.push(m);
                }
                j += 1;
            }
            classes.push(Class { name, indent, end_line: j, methods });
            i = j + 1;
        } else {
            i += 1;
        }
    }

    // The methods the implementation has: `Class.Method` headers.
    let mut implemented: Vec<(String, String)> = Vec::new();
    let mut impl_headers: Vec<(String, Method)> = Vec::new();
    for line in &lines[imp..] {
        if let Some(m) = header(line) {
            if let Some((cls, name)) = m.name.split_once('.') {
                implemented.push((cls.to_ascii_lowercase(), name.to_ascii_lowercase()));
                impl_headers.push((cls.to_string(), Method { keyword: m.keyword.clone(), name: name.to_string(), sig: m.sig.clone() }));
            }
        }
    }

    // Where the new text goes: the bodies before the implementation's last `end.` (or `initialization`),
    // the declarations before each class's `end;`.
    let mut inserts: Vec<(usize, String)> = Vec::new();
    let mut bodies = 0;
    let mut declarations = 0;
    let body_at = lines[imp..]
        .iter()
        .position(|l| l.trim().eq_ignore_ascii_case("initialization"))
        .map(|k| imp + k)
        .or_else(|| lines.iter().rposition(|l| l.trim().to_ascii_lowercase().starts_with("end.")))
        .ok_or("This unit has no end. to put the new bodies before")?;
    let mut bodies_text = String::new();
    for class in &classes {
        for m in &class.methods {
            let key = (class.name.to_ascii_lowercase(), m.name.to_ascii_lowercase());
            if !implemented.contains(&key) {
                bodies_text.push_str(&format!("{} {}.{}{};\nbegin\nend;\n\n", m.keyword, class.name, m.name, sig_gap(&m.sig)));
                bodies += 1;
            }
        }
    }
    if !bodies_text.is_empty() {
        inserts.push((body_at, bodies_text));
    }
    for (cls, m) in &impl_headers {
        let Some(class) = classes.iter().find(|c| c.name.eq_ignore_ascii_case(cls)) else { continue };
        let declared = class.methods.iter().any(|d| d.name.eq_ignore_ascii_case(&m.name));
        if !declared {
            let decl = format!("{}  {} {}{};\n", class.indent, m.keyword, m.name, sig_gap(&m.sig));
            inserts.push((class.end_line, decl));
            declarations += 1;
        }
    }

    // Emit the text with the inserts in place (several may share a line; they keep their order).
    let mut out = String::with_capacity(text.len() + 256);
    for (idx, line) in lines.iter().enumerate() {
        for (at, s) in &inserts {
            if *at == idx {
                out.push_str(s);
            }
        }
        out.push_str(line);
    }
    Ok(Completed { text: out, bodies, declarations })
}

/// A space before the signature when it has one (`(X: Integer)` follows the name directly; `: Integer` does not need one).
fn sig_gap(sig: &str) -> String {
    if sig.is_empty() {
        String::new()
    } else if sig.starts_with('(') || sig.starts_with(':') {
        sig.to_string()
    } else {
        format!(" {sig}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNIT: &str = "unit u;\n\ninterface\n\ntype\n  TFoo = class\n    procedure Bar(X: Integer); virtual;\n    function Baz: Integer;\n    procedure Extra;\n  end;\n\nimplementation\n\nprocedure TFoo.Bar(X: Integer);\nbegin\nend;\n\nprocedure TFoo.Undeclared(Y: string);\nbegin\nend;\n\nend.\n";

    #[test]
    fn missing_bodies_are_added_and_missing_declarations_too() {
        let done = complete(UNIT).unwrap();
        assert_eq!(done.bodies, 2, "Baz and Extra have no body");
        assert_eq!(done.declarations, 1, "Undeclared is not in the class");
        let t = &done.text;
        assert!(t.contains("function TFoo.Baz: Integer;\nbegin\nend;\n\nprocedure TFoo.Extra;\nbegin\nend;\n\nend.\n"), "{t}");
        assert!(t.contains("    procedure Extra;\n    procedure Undeclared(Y: string);\n  end;"), "{t}");
        assert_eq!(t.matches("procedure TFoo.Bar").count(), 1, "the existing body is not repeated");
    }

    #[test]
    fn a_complete_unit_changes_nothing() {
        let once = complete(UNIT).unwrap().text;
        let again = complete(&once).unwrap();
        assert_eq!((again.bodies, again.declarations), (0, 0));
        assert_eq!(again.text, once);
    }

    #[test]
    fn a_unit_without_an_implementation_is_refused() {
        assert!(complete("unit u;\ninterface\nimplementation\n").is_err(), "no end. to put the bodies before");
        assert!(complete("unit u;\ninterface\ntype TFoo = class end;\n").is_err());
    }
}

impl crate::App {
    /// Edit > Complete Class: the active unit gets its missing bodies and declarations. The change is unsaved,
    /// like typing.
    pub(crate) fn complete_class(&mut self, ctx: &eframe::egui::Context) {
        let Some(doc) = self.docs.get(self.active) else {
            self.note("Complete Class needs a unit open");
            return;
        };
        let is_unit = doc.path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("pas") || e.eq_ignore_ascii_case("pp"));
        if !is_unit {
            self.note("Complete Class works on a unit (.pas or .pp)");
            return;
        }
        let text = doc.text.clone();
        match complete(&text) {
            Ok(done) if done.bodies + done.declarations == 0 => self.note("Every method of the classes has its body and declaration"),
            Ok(done) => {
                let msg = format!("Added {} method bodies and {} declarations", done.bodies, done.declarations);
                crate::assistant::set_doc_text(&mut self.docs[self.active], done.text, false, Some(ctx));
                self.note(msg);
            }
            Err(e) => self.note(e),
        }
    }
}
