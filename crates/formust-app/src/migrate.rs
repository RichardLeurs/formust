//! The settings folder moved from the old app's name to Formust's. The first start copies
//! `~/Library/Application Support/Pascoda` to `~/Library/Application Support/Formust`, once. The old
//! folder is never changed or removed, so the old app keeps working. The Keychain has its own
//! fallback (see `formust_ai::Keystore::load`).

use std::io;
use std::path::{Path, PathBuf};

/// The folder the settings were kept in before the rename.
const OLD_FOLDER: &str = "Pascoda";
/// The folder the settings are kept in now.
const FOLDER: &str = "Formust";

/// The `Library/Application Support` folder of the user, or `None` without a home folder.
pub fn support_folder() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(Path::new(&home).join("Library/Application Support"))
}

/// Copies `support/Pascoda` to `support/Formust` when the new folder does not exist yet and the old
/// one does. Returns the number of files copied, or `None` when nothing was done. The copy is made
/// beside the new folder and renamed into place, so an interrupted copy never leaves a half-made
/// `Formust` folder that would stop the next start from trying again.
pub fn copy_settings(support: &Path) -> io::Result<Option<usize>> {
    let old = support.join(OLD_FOLDER);
    let new = support.join(FOLDER);
    if new.exists() || !old.is_dir() {
        return Ok(None);
    }
    let partial = support.join(format!("{FOLDER}.copying"));
    let _ = std::fs::remove_dir_all(&partial);
    let copied = match copy_dir(&old, &partial) {
        Ok(n) => n,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&partial);
            return Err(e);
        }
    };
    if let Err(e) = std::fs::rename(&partial, &new) {
        let _ = std::fs::remove_dir_all(&partial);
        return Err(e);
    }
    Ok(Some(copied))
}

/// Copies every file and folder under `from` into `to`, which must not exist. Symbolic links are
/// skipped: the settings are plain files. Returns the number of files copied.
fn copy_dir(from: &Path, to: &Path) -> io::Result<usize> {
    std::fs::create_dir(to)?;
    let mut count = 0;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            count += copy_dir(&entry.path(), &target)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &target)?;
            count += 1;
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh support folder with an old `Pascoda` folder in it.
    fn support(tag: &str) -> PathBuf {
        let support = crate::target_tmp().join(format!("migrate-{tag}"));
        let _ = std::fs::remove_dir_all(&support);
        let old = support.join("Pascoda");
        std::fs::create_dir_all(old.join("sub")).unwrap();
        std::fs::write(old.join("preferences"), "theme=dark\n").unwrap();
        std::fs::write(old.join("recent"), "file /p/a.rs\n").unwrap();
        std::fs::write(old.join("sub/openai-settings"), "base=http://x\n").unwrap();
        support
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn the_old_folder_is_copied_once_with_all_its_files() {
        let support = support("copy");
        assert_eq!(copy_settings(&support).unwrap(), Some(3));
        let new = support.join("Formust");
        assert_eq!(read(&new.join("preferences")), "theme=dark\n");
        assert_eq!(read(&new.join("recent")), "file /p/a.rs\n");
        assert_eq!(read(&new.join("sub/openai-settings")), "base=http://x\n");
        assert!(
            !support.join("Formust.copying").exists(),
            "no leftover copy"
        );
    }

    #[test]
    fn the_old_folder_is_left_exactly_as_it_was() {
        let support = support("keep");
        copy_settings(&support).unwrap();
        let old = support.join("Pascoda");
        assert_eq!(read(&old.join("preferences")), "theme=dark\n");
        assert_eq!(read(&old.join("sub/openai-settings")), "base=http://x\n");
        assert_eq!(std::fs::read_dir(&old).unwrap().count(), 3);
    }

    #[test]
    fn an_existing_new_folder_is_never_overwritten() {
        let support = support("exists");
        let new = support.join("Formust");
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join("preferences"), "theme=light\n").unwrap();
        assert_eq!(copy_settings(&support).unwrap(), None);
        assert_eq!(read(&new.join("preferences")), "theme=light\n");
        assert!(!new.join("recent").exists(), "nothing was copied");
    }

    #[test]
    fn a_second_start_does_nothing_even_when_the_old_folder_changes() {
        let support = support("second");
        assert_eq!(copy_settings(&support).unwrap(), Some(3));
        std::fs::write(support.join("Pascoda/preferences"), "theme=light\n").unwrap();
        assert_eq!(copy_settings(&support).unwrap(), None);
        assert_eq!(read(&support.join("Formust/preferences")), "theme=dark\n");
    }

    #[test]
    fn no_old_folder_means_nothing_to_copy() {
        let support = crate::target_tmp().join("migrate-none");
        let _ = std::fs::remove_dir_all(&support);
        std::fs::create_dir_all(&support).unwrap();
        assert_eq!(copy_settings(&support).unwrap(), None);
        assert!(!support.join("Formust").exists(), "no folder is made");
    }

    #[test]
    fn a_leftover_partial_copy_is_replaced_not_kept() {
        let support = support("partial");
        let partial = support.join("Formust.copying");
        std::fs::create_dir_all(&partial).unwrap();
        std::fs::write(partial.join("junk"), "from an interrupted copy").unwrap();
        assert_eq!(copy_settings(&support).unwrap(), Some(3));
        assert!(!support.join("Formust/junk").exists());
        assert!(!partial.exists());
    }

    #[test]
    fn symbolic_links_are_not_followed() {
        let support = support("links");
        std::os::unix::fs::symlink("/etc/hosts", support.join("Pascoda/link")).unwrap();
        assert_eq!(copy_settings(&support).unwrap(), Some(3));
        assert!(!support.join("Formust/link").exists());
    }
}
