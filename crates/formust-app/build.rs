//! Reads the IDE's identity from `[workspace.metadata.formust]` in the root `Cargo.toml` and makes
//! `name` and `copyright` available to the code as `FORMUST_NAME` and `FORMUST_COPYRIGHT` (see `about.rs`).
//! `packaging/bundle.sh` reads the same table for the app's `Info.plist`.

use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"),
    );
    let root = manifest.join("../../Cargo.toml");
    println!("cargo:rerun-if-changed={}", root.display());
    let text = std::fs::read_to_string(&root)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", root.display()));
    for (env, key) in [("FORMUST_NAME", "name"), ("FORMUST_COPYRIGHT", "copyright")] {
        let value = metadata_value(&text, key).unwrap_or_else(|| {
            panic!(
                "{key} is missing from [workspace.metadata.formust] in {}",
                root.display()
            )
        });
        println!("cargo:rustc-env={env}={value}");
    }
}

/// The value of `key = "..."` inside the `[workspace.metadata.formust]` table.
fn metadata_value(text: &str, key: &str) -> Option<String> {
    let mut in_table = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_table = line == "[workspace.metadata.formust]";
            continue;
        }
        if !in_table {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        if k.trim() != key {
            continue;
        }
        let v = v.trim();
        let inner = v.strip_prefix('"')?.split_once('"')?.0;
        return Some(inner.to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::metadata_value;

    #[test]
    fn reads_only_keys_of_the_formust_table() {
        let text = "[workspace]\nname = \"other\"\n\n[workspace.metadata.formust]\n# c\nname = \"Formust\"\ncopyright = \"Copyright © X 2026\"\n[profile]\nname = \"late\"\n";
        assert_eq!(metadata_value(text, "name").as_deref(), Some("Formust"));
        assert_eq!(
            metadata_value(text, "copyright").as_deref(),
            Some("Copyright © X 2026")
        );
        assert_eq!(metadata_value(text, "missing"), None);
    }
}
