//! `cargo metadata --no-deps --format-version 1`: the workspace root, the target folder, and the
//! packages with their targets. Nothing is resolved or downloaded.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::driver::{cargo_command, manifest_for, missing_cargo};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metadata {
    /// Relative file names in rustc's messages start here.
    pub workspace_root: PathBuf,
    pub target_directory: PathBuf,
    /// The workspace members, in cargo's order.
    pub packages: Vec<Package>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    pub manifest_path: PathBuf,
    pub targets: Vec<Target>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub name: String,
    /// `lib`, `bin`, `example`, `test`, `bench`, `custom-build`, ...
    pub kind: Vec<String>,
    pub src_path: PathBuf,
}

/// Reads the output of `cargo metadata --format-version 1`. `None` if it isn't that.
pub fn parse_metadata(json: &str) -> Option<Metadata> {
    let value: Value = serde_json::from_str(json).ok()?;
    let path = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(PathBuf::from);
    let string = |v: &Value, key: &str| {
        v.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let packages = value
        .get("packages")?
        .as_array()?
        .iter()
        .map(|p| Package {
            name: string(p, "name"),
            manifest_path: path(p, "manifest_path").unwrap_or_default(),
            targets: p
                .get("targets")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|t| Target {
                    name: string(t, "name"),
                    kind: t
                        .get("kind")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect(),
                    src_path: path(t, "src_path").unwrap_or_default(),
                })
                .collect(),
        })
        .collect();
    Some(Metadata {
        workspace_root: path(&value, "workspace_root")?,
        target_directory: path(&value, "target_directory").unwrap_or_default(),
        packages,
    })
}

/// Runs `cargo metadata --no-deps` for the package at `path` (a `Cargo.toml`, or a folder or file
/// inside a package; see [`manifest_for`]) and waits for it. `cargo` is [`crate::locate_cargo`]'s
/// when `None`. A failed run is an error with cargo's message.
pub fn metadata(cargo: Option<&Path>, path: &Path) -> std::io::Result<Metadata> {
    let cargo = match cargo {
        Some(c) => c.to_path_buf(),
        None => crate::locate_cargo().ok_or_else(missing_cargo)?,
    };
    let manifest = manifest_for(path)?;
    let output = metadata_command(&cargo, &manifest, &[])
        .output()
        .map_err(|e| crate::driver::spawn_error(&cargo, e))?;
    read_output(&output)
}

pub(crate) fn metadata_command(
    cargo: &Path,
    manifest: &Path,
    env: &[(std::ffi::OsString, std::ffi::OsString)],
) -> std::process::Command {
    let mut cmd = cargo_command(cargo, manifest, env);
    cmd.args([
        "metadata",
        "--no-deps",
        "--format-version",
        "1",
        "--manifest-path",
    ])
    .arg(manifest);
    cmd
}

pub(crate) fn read_output(output: &std::process::Output) -> std::io::Result<Metadata> {
    if !output.status.success() {
        return Err(std::io::Error::other(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    parse_metadata(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| std::io::Error::other("cargo metadata gave no usable output"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fixture_gives_root_folder_and_targets() {
        // Real `cargo metadata --no-deps` output of cargo 1.98.1, paths replaced by `/fixture`.
        let meta = parse_metadata(include_str!("../fixtures/metadata.json")).unwrap();
        assert_eq!(meta.workspace_root, Path::new("/fixture/targets"));
        assert_eq!(meta.target_directory, Path::new("/fixture/targets/target"));
        assert_eq!(meta.packages.len(), 1);
        let package = &meta.packages[0];
        assert_eq!(package.name, "targets");
        assert_eq!(
            package.manifest_path,
            Path::new("/fixture/targets/Cargo.toml")
        );
        let targets: Vec<(&str, &str)> = package
            .targets
            .iter()
            .map(|t| (t.kind[0].as_str(), t.name.as_str()))
            .collect();
        assert_eq!(
            targets,
            [
                ("lib", "targets"),
                ("bin", "tool"),
                ("example", "demo"),
                ("test", "adds")
            ]
        );
        assert_eq!(
            package.targets[1].src_path,
            Path::new("/fixture/targets/src/bin/tool.rs")
        );
    }

    #[test]
    fn other_text_is_not_metadata() {
        assert_eq!(parse_metadata(""), None);
        assert_eq!(parse_metadata("{\"packages\":[]}"), None);
        assert_eq!(parse_metadata("error: could not find `Cargo.toml`"), None);
    }
}
