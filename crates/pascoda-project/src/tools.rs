//! External tools (spec 39): the user's own commands, listed under Tools. Kept per user in
//! `~/Library/Application Support/Pascoda/tools`, one tool per line as four tab-separated fields (name,
//! command, arguments, working folder); tabs, line breaks and backslashes inside a field are escaped.
//!
//! A tool's arguments are split into words first and macros are expanded inside each word, so
//! `$(Selection)` stays one argument however many spaces it has. Nothing goes through a shell.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Tool {
    pub name: String,
    pub command: String,
    /// The arguments as typed; split into words by [`plan`].
    pub args: String,
    /// The folder the tool runs in; empty means the project folder.
    pub workdir: String,
}

/// What the IDE knows when a tool runs. A missing value expands to nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Context {
    /// The active file (`$(File)`, `$(Dir)`, `$(Name)`).
    pub file: Option<PathBuf>,
    pub project_dir: Option<PathBuf>,
    /// The program built for the active file (`$(Program)`).
    pub program: Option<PathBuf>,
    /// The compiled executable of that program (`$(Exe)`).
    pub exe: Option<PathBuf>,
    /// The selected text (`$(Selection)`).
    pub selection: String,
    /// The line of the caret, 1-based (`$(Line)`).
    pub line: u32,
}

/// A tool ready to run: its command and arguments, its working folder, and the macros it did not know.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub command: String,
    pub args: Vec<String>,
    pub workdir: PathBuf,
    pub unknown: Vec<String>,
}

/// The macros a tool may use, for the editor's help line.
pub const MACROS: &str = "$(File) $(Dir) $(Name) $(ProjectDir) $(Program) $(Exe) $(Selection) $(Line)";

/// The file that keeps the tools, or `None` without a home folder.
pub fn file() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/Pascoda/tools"))
}

fn escape(field: &str) -> String {
    field.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n")
}

fn unescape(field: &str) -> String {
    let mut out = String::new();
    let mut chars = field.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// The tools in `text`, in file order. A line without four fields is skipped.
pub fn parse(text: &str) -> Vec<Tool> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<String> = line.split('\t').map(unescape).collect();
            match fields.as_slice() {
                [name, command, args, workdir] => Some(Tool {
                    name: name.clone(),
                    command: command.clone(),
                    args: args.clone(),
                    workdir: workdir.clone(),
                }),
                _ => None,
            }
        })
        .collect()
}

pub fn to_text(tools: &[Tool]) -> String {
    tools
        .iter()
        .map(|t| {
            format!(
                "{}\t{}\t{}\t{}\n",
                escape(&t.name),
                escape(&t.command),
                escape(&t.args),
                escape(&t.workdir)
            )
        })
        .collect()
}

/// The tools every new installation starts with. They are ordinary tools: the user can edit, reorder or
/// remove them in Configure Tools. The syntax check uses the compiler Pascoda itself finds, since a
/// Finder-launched app may not have `fpc` on its PATH.
pub fn defaults() -> Vec<Tool> {
    let fpc = pascoda_fpc::locate_fpc().map_or_else(|| "fpc".to_string(), |p| p.display().to_string());
    let tool = |name: &str, command: &str, args: &str, workdir: &str| Tool {
        name: name.into(),
        command: command.into(),
        args: args.into(),
        workdir: workdir.into(),
    };
    vec![
        tool("Syntax Check", &fpc, "-s $(File)", "$(Dir)"),
        tool("Line Count", "wc", "-l $(File)", "$(Dir)"),
        tool("Git Status", "git", "status", "$(ProjectDir)"),
        tool("Open Project Folder", "open", "$(ProjectDir)", ""),
    ]
}

/// The file that lists the default tools already offered (one name per line), so a default the user
/// removed is not added again. It sits next to the tools file.
fn seeded_file() -> Option<PathBuf> {
    file().map(|f| f.with_file_name("tools-defaults"))
}

/// The saved tools. Each default tool is added once, the first time it is loaded (when its name is not
/// already listed); after that the user alone decides, through Configure Tools. A file that cannot be
/// read gives no tools.
pub fn load() -> Vec<Tool> {
    let (Some(path), Some(marker)) = (file(), seeded_file()) else { return Vec::new() };
    let mut tools = match std::fs::read_to_string(&path) {
        Ok(text) => parse(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(_) => return Vec::new(),
    };
    let mut offered: Vec<String> = std::fs::read_to_string(&marker)
        .map(|t| t.lines().map(str::to_string).collect())
        .unwrap_or_default();
    let mut added = false;
    for d in defaults() {
        if offered.contains(&d.name) {
            continue;
        }
        if !tools.iter().any(|t| t.name == d.name) {
            tools.push(d.clone());
            added = true;
        }
        offered.push(d.name.clone());
    }
    if added || !marker.exists() {
        let _ = save(&tools);
        if let Some(dir) = marker.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&marker, offered.join("\n") + "\n");
    }
    tools
}

/// Saves the tools, creating the folder when needed.
pub fn save(tools: &[Tool]) -> std::io::Result<()> {
    let Some(path) = file() else {
        return Err(std::io::Error::other("no home folder"));
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, to_text(tools))
}

/// The words of an argument string: separated by spaces, with `"..."` and `'...'` keeping their spaces.
/// An empty pair of quotes is an empty word.
pub fn split_args(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    for c in text.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => word.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                started = true;
            }
            None if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            None => {
                word.push(c);
                started = true;
            }
        }
    }
    if started {
        words.push(word);
    }
    words
}

/// The value of one macro name, or `None` when the name is not one of [`MACROS`].
fn value(name: &str, ctx: &Context) -> Option<String> {
    let path = |p: &Option<PathBuf>| p.as_ref().map_or_else(String::new, |p| p.display().to_string());
    Some(match name {
        "File" => path(&ctx.file),
        "Dir" => ctx.file.as_deref().and_then(Path::parent).map_or_else(String::new, |p| p.display().to_string()),
        "Name" => ctx
            .file
            .as_deref()
            .and_then(Path::file_stem)
            .map_or_else(String::new, |s| s.to_string_lossy().into_owned()),
        "ProjectDir" => path(&ctx.project_dir),
        "Program" => path(&ctx.program),
        "Exe" => path(&ctx.exe),
        "Selection" => ctx.selection.clone(),
        "Line" => ctx.line.to_string(),
        _ => return None,
    })
}

/// Replaces each `$(Name)` in `text`. An unknown name stays as written and is added to `unknown`; an unclosed
/// `$(` stays too.
pub fn expand(text: &str, ctx: &Context, unknown: &mut Vec<String>) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("$(") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 2..];
        let Some(close) = after.find(')') else {
            out.push_str(&rest[at..]);
            return out;
        };
        let name = &after[..close];
        match value(name, ctx) {
            Some(v) => out.push_str(&v),
            None => {
                unknown.push(name.to_string());
                out.push_str(&rest[at..at + 2 + close + 1]);
            }
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

/// The command, arguments and working folder of `tool` for `ctx`. The working folder defaults to the project.
pub fn plan(tool: &Tool, ctx: &Context) -> Plan {
    let mut unknown = Vec::new();
    let command = expand(tool.command.trim(), ctx, &mut unknown);
    let args = split_args(&tool.args)
        .iter()
        .map(|word| expand(word, ctx, &mut unknown))
        .collect();
    let dir = if tool.workdir.trim().is_empty() {
        "$(ProjectDir)".to_string()
    } else {
        tool.workdir.trim().to_string()
    };
    let workdir = PathBuf::from(expand(&dir, ctx, &mut unknown));
    Plan { command, args, workdir, unknown }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Context {
        Context {
            file: Some(PathBuf::from("/p/src/unit1.pas")),
            project_dir: Some(PathBuf::from("/p")),
            program: Some(PathBuf::from("/p/app.lpr")),
            exe: Some(PathBuf::from("/p/app")),
            selection: "two words".into(),
            line: 7,
        }
    }

    fn tool(command: &str, args: &str, workdir: &str) -> Tool {
        Tool { name: "t".into(), command: command.into(), args: args.into(), workdir: workdir.into() }
    }

    #[test]
    fn saved_tools_come_back_with_tabs_quotes_and_line_breaks() {
        let tools = vec![
            Tool { name: "Tab\there".into(), command: "echo".into(), args: "a \"b c\"\\".into(), workdir: String::new() },
            Tool { name: "Two\nlines".into(), command: "/bin/ls".into(), args: "-l".into(), workdir: "/tmp".into() },
        ];
        assert_eq!(parse(&to_text(&tools)), tools);
        assert!(parse("broken line\n").is_empty(), "a line without four fields is skipped");
    }

    #[test]
    fn every_macro_expands_and_a_missing_value_is_empty() {
        let mut unknown = Vec::new();
        assert_eq!(expand("$(File)", &ctx(), &mut unknown), "/p/src/unit1.pas");
        assert_eq!(expand("$(Dir)|$(Name)", &ctx(), &mut unknown), "/p/src|unit1");
        assert_eq!(expand("$(ProjectDir)/$(Program)", &ctx(), &mut unknown), "/p//p/app.lpr");
        assert_eq!(expand("$(Exe)@$(Line)", &ctx(), &mut unknown), "/p/app@7");
        assert_eq!(expand("[$(Selection)]", &ctx(), &mut unknown), "[two words]");
        assert_eq!(expand("[$(File)]", &Context::default(), &mut unknown), "[]");
        assert!(unknown.is_empty());
    }

    #[test]
    fn an_unknown_macro_stays_as_written_and_is_reported() {
        let mut unknown = Vec::new();
        assert_eq!(expand("a $(Nope) b $(File", &ctx(), &mut unknown), "a $(Nope) b $(File");
        assert_eq!(unknown, ["Nope"]);
    }

    #[test]
    fn quoted_words_keep_their_spaces_and_an_empty_pair_is_a_word() {
        assert_eq!(split_args("-o 'a b' \"c d\" e"), ["-o", "a b", "c d", "e"]);
        assert_eq!(split_args("x \"\" y"), ["x", "", "y"]);
        assert!(split_args("   ").is_empty());
    }

    #[test]
    fn the_selection_is_one_argument_however_many_words_it_has() {
        let p = plan(&tool("/bin/echo", "$(Selection) end", ""), &ctx());
        assert_eq!(p.args, ["two words", "end"]);
        assert_eq!(p.workdir, PathBuf::from("/p"), "the working folder defaults to the project");
        assert!(p.unknown.is_empty());
    }

    #[test]
    fn a_plan_reports_unknown_macros_in_command_args_and_folder() {
        let p = plan(&tool("$(Bad)", "$(Worse)", "$(Gone)"), &ctx());
        assert_eq!(p.unknown, ["Bad", "Worse", "Gone"]);
    }
}

#[cfg(test)]
mod default_tools_tests {
    use super::*;

    #[test]
    fn the_defaults_are_saved_as_ordinary_tools_and_read_back() {
        let tools = defaults();
        assert_eq!(tools.len(), 4);
        assert_eq!(parse(&to_text(&tools)), tools, "they survive the tools file unchanged");
        let syntax = &tools[0];
        assert_eq!(syntax.name, "Syntax Check");
        assert_eq!(split_args(&syntax.args), ["-s", "$(File)"]);
        assert_eq!(syntax.workdir, "$(Dir)");
    }
}
