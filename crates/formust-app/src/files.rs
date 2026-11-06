//! Creating and deleting files and folders in the project tree. UI-free: the navigator decides
//! when to call these and what to do with the tabs and designers of a deleted item.

use formust_rust::{FileKind, Header, file_kind, form_path_for, source_path_for};
use std::path::{Path, PathBuf};

/// The text files the new-file menu creates besides Rust sources: notes, settings and scripts.
/// `FILE_EXTENSIONS` must hold all of them (a test checks it).
#[cfg(test)]
pub const OTHER_TEXT_EXTENSIONS: &[&str] = &[
    "txt", "md", "markdown", "cfg", "ini", "toml", "json", "xml", "yml", "yaml", "sh",
];

/// The kinds a new file can be (the tree lists them, and a new file must be one of these, or it
/// would be created and then never shown). A form file `name.frm.rs` has the extension `rs`, so it
/// needs no entry of its own; `is_listed` also accepts the whole suffix.
pub const FILE_EXTENSIONS: &[&str] = &[
    "rs", "txt", "md", "markdown", "cfg", "ini", "toml", "json", "xml", "yml", "yaml", "sh",
];

/// What a new file of this kind starts with: a shebang for a shell script, and the minimum a JSON
/// or XML document needs. Everything else (Rust sources included) starts empty.
fn starter_text(ext: &str) -> String {
    match ext {
        "sh" => "#!/bin/sh\n".into(),
        "json" => "{}\n".into(),
        "xml" => "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n".into(),
        _ => String::new(),
    }
}

/// The files the tree lists: Rust sources and form files, Cargo's manifests (`Cargo.toml` is a
/// `.toml`), and other text files, which are read but not created here.
pub const TREE_EXTENSIONS: &[&str] = &[
    "rs", "toml", "md", "markdown", "txt", "cfg", "ini", "json", "xml", "yml", "yaml", "sh",
];

/// Folders the tree hides (and so cannot create either). `lib` is a Rust module folder, so it shows.
pub const HIDDEN_DIRS: &[&str] = &["target", "vendor"];

/// Whether the tree lists a file: by its extension, or as a form file (its name ends `.frm.rs`).
pub fn is_listed(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if name.ends_with(formust_rust::FORM_SUFFIX) {
        return true;
    }
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| TREE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// A name typed for a new file or folder, trimmed and checked for the characters a path cannot
/// hold and for names the tree would hide.
pub fn check_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Type a name first".into());
    }
    if name.contains(['/', ':']) {
        return Err("A name cannot contain / or :".into());
    }
    if name.starts_with('.') {
        return Err("A name cannot start with a dot".into());
    }
    Ok(name)
}

/// Creates the file `name` in `dir`, with the starting text of its kind (empty for most). A name
/// without an extension gets `.rs`; an extension the tree does not create is refused. An existing
/// file is never overwritten.
pub fn new_file(dir: &Path, name: &str) -> Result<PathBuf, String> {
    let name = check_name(name)?;
    let name = if Path::new(name).extension().is_none() {
        format!("{name}.rs")
    } else {
        name.to_string()
    };
    let ext = Path::new(&name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !FILE_EXTENSIONS.contains(&ext.as_str()) {
        return Err(format!(
            "The project lists only {} files",
            FILE_EXTENSIONS
                .iter()
                .map(|e| format!(".{e}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let path = dir.join(&name);
    if path.exists() {
        return Err(format!("“{name}” already exists"));
    }
    use std::io::Write;
    let mut file = std::fs::File::create_new(&path).map_err(|e| e.to_string())?;
    file.write_all(starter_text(&ext).as_bytes())
        .map_err(|e| e.to_string())?;
    Ok(path)
}

/// Creates the folder `name` in `dir`; an existing item is never replaced.
pub fn new_folder(dir: &Path, name: &str) -> Result<PathBuf, String> {
    let name = check_name(name)?;
    if name.ends_with(".app") || HIDDEN_DIRS.contains(&name.to_ascii_lowercase().as_str()) {
        return Err(format!("The tree hides folders named “{name}”"));
    }
    let path = dir.join(name);
    if path.exists() {
        return Err(format!("“{name}” already exists"));
    }
    std::fs::create_dir(&path).map_err(|e| e.to_string())?;
    Ok(path)
}

/// What a delete would remove: 1 for a file, the file and folder count below a folder.
pub fn count_items(path: &Path) -> usize {
    if !path.is_dir() {
        return 1;
    }
    let Ok(read) = std::fs::read_dir(path) else {
        return 1;
    };
    1 + read
        .filter_map(Result::ok)
        .map(|e| count_items(&e.path()))
        .sum::<usize>()
}

/// Moves a file or a whole folder to the Trash, so the user can take it back. Only items strictly
/// inside `root` can go: the project folder itself, and anything outside it, are refused. The
/// caller asks the user first.
#[cfg(not(test))]
pub fn delete(path: &Path, root: &Path) -> Result<(), String> {
    delete_with(path, root, |p| trash::delete(p).map_err(|e| e.to_string()))
}

/// Tests remove for real instead of using the Trash, so they do not fill the user's Trash.
#[cfg(test)]
pub fn delete(path: &Path, root: &Path) -> Result<(), String> {
    delete_with(path, root, remove_from_disk)
}

/// [`delete`] with the step that actually removes the item.
pub fn delete_with(
    path: &Path,
    root: &Path,
    mut remove: impl FnMut(&Path) -> Result<(), String>,
) -> Result<(), String> {
    if path == root || !path.starts_with(root) {
        return Err("Only items inside the project can be deleted".into());
    }
    let with = companions(path);
    remove(path)?;
    for other in &with {
        remove(other)?;
    }
    Ok(())
}

/// The file a source's form is kept in: `x.frm.rs` beside `x.rs`, or `forms/x.frm.rs` in a Cargo
/// target folder (see `formust_rust::paths`). Only when that file exists.
fn form_of(source: &Path) -> Option<PathBuf> {
    form_path_for(source, |p| p.is_file())
        .map(|f| f.path)
        .filter(|f| f.is_file())
}

/// The source a form file belongs to: the one its header names, else the one beside it. Only when
/// that file exists.
fn source_of(form: &Path) -> Option<PathBuf> {
    let header = std::fs::read_to_string(form)
        .ok()
        .and_then(|text| text.lines().find_map(Header::parse));
    source_path_for(form, header.as_ref(), |p| p.is_file()).filter(|s| s.is_file())
}

/// The files that go with `path` when it is deleted: a source's form, and a form's source. Only
/// those that exist.
pub fn companions(path: &Path) -> Vec<PathBuf> {
    if !path.is_file() {
        return Vec::new();
    }
    match file_kind(path) {
        FileKind::Source => form_of(path).into_iter().collect(),
        FileKind::Form => source_of(path).into_iter().collect(),
        FileKind::Other => Vec::new(),
    }
}

/// Everything a deletion of `path` removes: the item itself and its companions.
pub fn removed_with(path: &Path) -> Vec<PathBuf> {
    let mut all = vec![path.to_path_buf()];
    all.extend(companions(path));
    all
}

/// Removes a file or a whole folder from disk, for the tests.
#[cfg(test)]
fn remove_from_disk(path: &Path) -> Result<(), String> {
    let result = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    result.map_err(|e| e.to_string())
}

/// Renames a file or a folder inside its folder. A file given a name without an extension keeps its
/// own; the new extension must be one the tree lists. A source moves with its form, whose header
/// is updated to the new name. Returns the moves made, or nothing when the name is unchanged. The
/// project folder itself and anything outside `root` are refused, and so is a name that is taken.
pub fn rename(path: &Path, new_name: &str, root: &Path) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    if path == root || !path.starts_with(root) {
        return Err("Only items inside the project can be renamed".into());
    }
    let name = check_name(new_name)?;
    let name = match (
        path.is_file(),
        Path::new(name).extension(),
        path.extension(),
    ) {
        (true, None, Some(old)) => format!("{name}.{}", old.to_string_lossy()),
        _ => name.to_string(),
    };
    if path.is_file() {
        let ext = Path::new(&name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !TREE_EXTENSIONS.contains(&ext.as_str()) {
            return Err("The project lists only Rust, Cargo and text files".into());
        }
    } else if name.ends_with(".app") || HIDDEN_DIRS.contains(&name.to_ascii_lowercase().as_str()) {
        return Err(format!("The tree hides folders named “{name}”"));
    }
    let parent = path.parent().ok_or("The item has no folder")?;
    relocate(path, parent, &name)
}

/// Moves a file or a folder into another folder of the project, keeping its name. A source moves
/// with its form. Returns the moves made; moving into the folder it is already in changes nothing.
/// The project folder, a folder into itself, folders outside the project and a taken name are refused.
pub fn move_into(path: &Path, dir: &Path, root: &Path) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    if path == root || !path.starts_with(root) {
        return Err("Only items inside the project can be moved".into());
    }
    if !dir.starts_with(root) || !dir.is_dir() {
        return Err("Items can only be moved into a folder of the project".into());
    }
    if dir.starts_with(path) {
        return Err("A folder cannot be moved into itself".into());
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or("The item has no name")?;
    relocate(path, dir, &name)
}

/// Puts `path` at `dir/name`, with the form of a source beside it (or in its target folder's `forms`
/// folder). Moves nothing when the target is the path itself.
fn relocate(path: &Path, dir: &Path, name: &str) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    let target = dir.join(name);
    if target == path {
        return Ok(Vec::new());
    }
    if target.exists() && !same_item(&target, path) {
        return Err(format!("“{name}” already exists"));
    }
    let mut moves = vec![(path.to_path_buf(), target.clone())];
    let form_target = if file_kind(path) == FileKind::Source {
        form_of(path).map(|from| {
            let to = form_path_for(&target, |p| p.is_file())
                .map(|f| f.path)
                .unwrap_or_default();
            (from, to)
        })
    } else {
        None
    };
    if let Some((from, to)) = &form_target {
        if to.exists() && !same_item(to, from) {
            return Err(format!("“{}” already exists", to.display()));
        }
        moves.push((from.clone(), to.clone()));
    }
    let mut done: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (from, to) in &moves {
        let made = match to.parent() {
            Some(parent) => {
                std::fs::create_dir_all(parent).and_then(|()| std::fs::rename(from, to))
            }
            None => std::fs::rename(from, to),
        };
        if let Err(e) = made {
            // Put back what was already moved, so a failed rename leaves the project as it was.
            for (f, t) in done.iter().rev() {
                let _ = std::fs::rename(t, f);
            }
            return Err(e.to_string());
        }
        done.push((from.clone(), to.clone()));
    }
    if let Some((_, to)) = &form_target {
        retarget_header(to, &target)?;
    }
    Ok(moves)
}

/// Points a form's header at its source's new place: the header's file field is relative to the
/// form's folder, so it changes when the form moves to a `forms` folder, or the source is renamed.
fn retarget_header(form: &Path, source: &Path) -> Result<(), String> {
    let Some(header_file) = form_path_for(source, |p| p.is_file()).map(|f| f.header_file) else {
        return Ok(());
    };
    let text = std::fs::read_to_string(form).map_err(|e| e.to_string())?;
    let mut changed = false;
    let out: String = text
        .split_inclusive('\n')
        .map(|line| match Header::parse(line) {
            Some(mut header) if !changed => {
                changed = true;
                header.file = header_file.clone();
                let eol = if line.ends_with('\n') { "\n" } else { "" };
                format!("{}{eol}", header.to_line())
            }
            _ => line.to_string(),
        })
        .collect();
    if changed && out != text {
        std::fs::write(form, out).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Whether two paths name the same file or folder (they may differ in case on macOS).
fn same_item(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Where `path` ends up when `from` is moved to `to`, if it is `from` or inside it.
pub fn moved(path: &Path, from: &Path, to: &Path) -> Option<PathBuf> {
    // `join("")` would add a trailing slash, which makes a file read as a folder (ENOTDIR).
    path.strip_prefix(from).ok().map(|rest| {
        if rest.as_os_str().is_empty() {
            to.to_path_buf()
        } else {
            to.join(rest)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("formust-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A form file with a header for `source`, as the designer writes it.
    fn form_text(source: &str) -> String {
        format!(
            "// formust 1 | {source} | Login::ui | src-hash 00000001\n#[derive(Default)] pub struct LoginForm;\n"
        )
    }

    #[test]
    fn names_are_checked() {
        assert!(check_name("  ").is_err());
        assert!(check_name("a/b").is_err());
        assert!(check_name("a:b").is_err());
        assert!(check_name(".hidden").is_err());
        assert_eq!(check_name(" main2 ").unwrap(), "main2");
    }

    #[test]
    fn the_tree_lists_rust_sources_form_files_and_cargo_manifests() {
        for path in [
            "src/main.rs",
            "forms/login.frm.rs",
            "login.frm.rs",
            "Cargo.toml",
            "README.md",
            "notes.txt",
        ] {
            assert!(is_listed(Path::new(path)), "{path} is listed");
        }
        for path in ["Cargo.lock", "app.exe", "libx.a"] {
            assert!(!is_listed(Path::new(path)), "{path} is not listed");
        }
        for ext in FILE_EXTENSIONS {
            assert!(TREE_EXTENSIONS.contains(ext), "a created {ext} is listed");
        }
        for ext in OTHER_TEXT_EXTENSIONS {
            assert!(FILE_EXTENSIONS.contains(ext), "{ext} can be created");
            assert!(TREE_EXTENSIONS.contains(ext), "{ext} is listed");
        }
    }

    #[test]
    fn lib_is_shown_and_target_is_hidden() {
        assert!(
            !HIDDEN_DIRS.contains(&"lib"),
            "a Rust module folder is shown"
        );
        assert!(HIDDEN_DIRS.contains(&"target"));
        let d = scratch("hidden-dirs");
        assert!(new_folder(&d, "lib").is_ok());
        assert!(
            new_folder(&d, "target").is_err(),
            "build output stays hidden"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn new_files_start_with_what_their_kind_needs() {
        let d = scratch("starters");
        assert!(
            std::fs::read_to_string(new_file(&d, "main.rs").unwrap())
                .unwrap()
                .is_empty(),
            "an empty crate root is valid Rust"
        );
        assert_eq!(
            std::fs::read_to_string(new_file(&d, "run.sh").unwrap()).unwrap(),
            "#!/bin/sh\n"
        );
        assert_eq!(
            std::fs::read_to_string(new_file(&d, "app.json").unwrap()).unwrap(),
            "{}\n"
        );
        assert!(
            std::fs::read_to_string(new_file(&d, "readme.md").unwrap())
                .unwrap()
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn new_file_adds_rs_and_refuses_unlisted_or_existing() {
        let d = scratch("file");
        let p = new_file(&d, "unit2").unwrap();
        assert_eq!(p.file_name().unwrap(), "unit2.rs");
        assert!(p.is_file());
        assert!(new_file(&d, "unit2.rs").is_err(), "never overwrite");
        assert!(new_file(&d, "notes.exe").is_err(), "the tree would hide it");
        assert!(
            new_file(&d, "Cargo.lock").is_err(),
            "cargo writes its own lock file"
        );
        assert!(
            new_file(&d, "lib.RS").is_ok(),
            "extensions are case-insensitive"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn new_folder_refuses_hidden_names_and_existing_items() {
        let d = scratch("dir");
        let p = new_folder(&d, "sub").unwrap();
        assert!(p.is_dir());
        assert!(new_folder(&d, "sub").is_err());
        assert!(new_folder(&d, "Foo.app").is_err(), "bundles are hidden");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn delete_removes_files_and_folders_but_not_the_project() {
        let root = scratch("delete");
        let f = new_file(&root, "a.rs").unwrap();
        let dir = new_folder(&root, "sub").unwrap();
        new_file(&dir, "b.rs").unwrap();
        assert_eq!(count_items(&dir), 2);

        assert!(delete(&root, &root).is_err(), "the project folder stays");
        assert!(delete(&root.join("..").join("outside"), &root).is_err());
        delete(&f, &root).unwrap();
        assert!(!f.exists());
        delete(&dir, &root).unwrap();
        assert!(!dir.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The real Trash, so this is run by hand (`cargo test -- --ignored real_trash`): it moves
    /// one scratch file to the user's Trash and checks that the original is gone.
    #[test]
    #[ignore = "moves a file to the real Trash"]
    fn real_trash_moves_the_item() {
        let root = scratch("real-trash");
        let file = root.join("formust-trash-check.rs");
        std::fs::write(&file, "fn check() {}\n").unwrap();
        trash::delete(&file).expect("the Trash accepts the file");
        assert!(!file.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rename_keeps_the_extension_and_refuses_taken_names() {
        let root = scratch("rename-file");
        let f = new_file(&root, "unit2").unwrap();
        let moves = rename(&f, "unit3", &root).unwrap();
        assert_eq!(moves, vec![(f.clone(), root.join("unit3.rs"))]);
        assert!(!f.exists() && root.join("unit3.rs").is_file());
        new_file(&root, "taken").unwrap();
        assert!(rename(&root.join("unit3.rs"), "taken.rs", &root).is_err());
        assert!(
            rename(&root.join("unit3.rs"), "notes.exe", &root).is_err(),
            "the tree would hide it"
        );
        assert!(
            rename(&root.join("unit3.rs"), "unit3.rs", &root)
                .unwrap()
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_source_moves_with_its_form_and_the_form_header_follows_the_name() {
        let root = scratch("rename-pair");
        std::fs::write(root.join("login.rs"), "pub struct Login;\n").unwrap();
        std::fs::write(root.join("login.frm.rs"), form_text("login.rs")).unwrap();
        let moves = rename(&root.join("login.rs"), "sign_in", &root).unwrap();
        assert_eq!(moves.len(), 2);
        assert!(root.join("sign_in.rs").is_file() && root.join("sign_in.frm.rs").is_file());
        assert!(!root.join("login.frm.rs").exists());
        let form = std::fs::read_to_string(root.join("sign_in.frm.rs")).unwrap();
        assert!(
            form.starts_with("// formust 1 | sign_in.rs | Login::ui |"),
            "{form}"
        );
        assert!(
            form.ends_with("pub struct LoginForm;\n"),
            "the body is untouched: {form}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_form_file_renamed_alone_leaves_its_source_alone() {
        let root = scratch("rename-form-only");
        std::fs::write(root.join("login.rs"), "pub struct Login;\n").unwrap();
        std::fs::write(root.join("login.frm.rs"), form_text("login.rs")).unwrap();
        let moves = rename(&root.join("login.frm.rs"), "copy.frm.rs", &root).unwrap();
        assert_eq!(moves.len(), 1);
        assert!(root.join("login.rs").is_file(), "the source stays");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_source_moves_into_a_folder_with_its_form() {
        let root = scratch("move-pair");
        let dir = new_folder(&root, "sub").unwrap();
        std::fs::write(root.join("login.rs"), "pub struct Login;\n").unwrap();
        std::fs::write(root.join("login.frm.rs"), form_text("login.rs")).unwrap();
        let moves = move_into(&root.join("login.rs"), &dir, &root).unwrap();
        assert_eq!(moves.len(), 2);
        assert!(dir.join("login.rs").is_file() && dir.join("login.frm.rs").is_file());
        assert!(!root.join("login.rs").exists() && !root.join("login.frm.rs").exists());
        let form = std::fs::read_to_string(dir.join("login.frm.rs")).unwrap();
        assert!(form.starts_with("// formust 1 | login.rs | "), "{form}");
        assert!(
            move_into(&dir.join("login.rs"), &dir, &root)
                .unwrap()
                .is_empty(),
            "already there"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A source in a Cargo target folder keeps its form in `forms/`, where Cargo does not build it.
    #[test]
    fn a_source_in_a_cargo_target_folder_keeps_its_form_in_forms() {
        let root = scratch("target-folder");
        let bin = root.join("src").join("bin");
        std::fs::create_dir_all(bin.join("forms")).unwrap();
        std::fs::write(bin.join("tool.rs"), "fn main() {}\n").unwrap();
        std::fs::write(
            bin.join("forms").join("tool.frm.rs"),
            form_text("../tool.rs"),
        )
        .unwrap();
        let moves = rename(&bin.join("tool.rs"), "tool2.rs", &root).unwrap();
        assert_eq!(moves.len(), 2);
        assert!(bin.join("tool2.rs").is_file());
        assert!(
            bin.join("forms").join("tool2.frm.rs").is_file(),
            "the form follows its source"
        );
        assert!(!bin.join("forms").join("tool.frm.rs").exists());
        let form = std::fs::read_to_string(bin.join("forms").join("tool2.frm.rs")).unwrap();
        assert!(form.starts_with("// formust 1 | ../tool2.rs | "), "{form}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn other_text_files_are_listed_and_can_be_renamed_to() {
        let root = scratch("other-text");
        std::fs::write(root.join("notes.txt"), "hello\n").unwrap();
        std::fs::write(root.join("readme.md"), "# hi\n").unwrap();
        std::fs::write(root.join("junk.o"), "x").unwrap();
        let names: Vec<String> = crate::shell::listing(&root)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(
            names.contains(&"notes.txt".to_string()) && names.contains(&"readme.md".to_string())
        );
        assert!(
            !names.contains(&"junk.o".to_string()),
            "compiled files stay hidden"
        );
        rename(&root.join("notes.txt"), "todo.md", &root).unwrap();
        assert!(root.join("todo.md").is_file());
        assert!(rename(&root.join("todo.md"), "bin.o", &root).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_move_refuses_a_taken_name_a_folder_into_itself_and_outside_the_project() {
        let root = scratch("move-refuse");
        let outer = new_folder(&root, "outer").unwrap();
        let inner = new_folder(&outer, "inner").unwrap();
        new_file(&inner, "a.rs").unwrap();
        new_file(&root, "a.rs").unwrap();
        let root_a = root.join("a.rs");
        assert!(
            move_into(&root_a, &inner, &root).is_err(),
            "inner already has a.rs"
        );
        assert!(move_into(&outer, &inner, &root).is_err(), "into itself");
        assert!(
            move_into(&root, &outer, &root).is_err(),
            "the project folder stays"
        );
        assert!(
            move_into(&root_a, &std::env::temp_dir(), &root).is_err(),
            "outside the project"
        );
        assert!(root_a.is_file(), "a refused move changes nothing");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn deleting_a_form_takes_its_source_and_deleting_a_source_takes_its_form() {
        let root = scratch("delete-pair");
        std::fs::write(root.join("login.rs"), "pub struct Login;\n").unwrap();
        std::fs::write(root.join("login.frm.rs"), form_text("login.rs")).unwrap();
        std::fs::write(root.join("other.rs"), "pub struct Other;\n").unwrap();
        std::fs::write(root.join("other.frm.rs"), form_text("other.rs")).unwrap();
        assert_eq!(
            removed_with(&root.join("login.frm.rs")).len(),
            2,
            "the form and its source"
        );
        delete_with(&root.join("login.frm.rs"), &root, remove_from_disk).unwrap();
        assert!(!root.join("login.rs").exists() && !root.join("login.frm.rs").exists());
        delete_with(&root.join("other.rs"), &root, remove_from_disk).unwrap();
        assert!(
            !root.join("other.frm.rs").exists(),
            "a source takes its form"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_renames_with_its_contents_and_the_project_cannot() {
        let root = scratch("rename-dir");
        let dir = new_folder(&root, "sub").unwrap();
        new_file(&dir, "inner.rs").unwrap();
        rename(&dir, "renamed", &root).unwrap();
        assert!(root.join("renamed/inner.rs").is_file());
        assert!(
            rename(&root, "elsewhere", &root).is_err(),
            "the project folder stays"
        );
        assert!(
            rename(&root.join("renamed"), "target", &root).is_err(),
            "the tree hides target"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_case_change_alone_is_a_rename() {
        let root = scratch("rename-case");
        let f = new_file(&root, "lower.rs").unwrap();
        rename(&f, "Lower.rs", &root).unwrap();
        let names: Vec<String> = std::fs::read_dir(&root)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["Lower.rs".to_string()]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn moved_maps_a_path_inside_a_renamed_folder() {
        let from = Path::new("/p/sub");
        let to = Path::new("/p/new");
        assert_eq!(
            moved(Path::new("/p/sub/a.rs"), from, to),
            Some(PathBuf::from("/p/new/a.rs"))
        );
        assert_eq!(moved(from, from, to), Some(PathBuf::from("/p/new")));
        // `Path` equality ignores a trailing slash, so compare the text too: "/p/new/" is a folder path.
        assert_eq!(moved(from, from, to).unwrap().to_string_lossy(), "/p/new");
        assert_eq!(moved(Path::new("/p/subway.rs"), from, to), None);
    }
}
