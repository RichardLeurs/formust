//! Export a finished program: an optimised build packaged as a `.app` (GUI programs) or a plain
//! executable (console programs), optionally inside a `.dmg`.

use crate::lcl::{self, BundleInfo, Lcl};
use crate::winres;
use crate::{Arch, BuildResult, Mode, Plan, Progress, build_streaming_for};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub struct Exported {
    /// What the compiler said. When it failed, `product` is `None`.
    pub build: BuildResult,
    /// `<name>.app`, or the executable of a console program.
    pub product: Option<PathBuf>,
    /// `<name>.dmg` when asked for.
    pub dmg: Option<PathBuf>,
}

/// A name that is a safe file and bundle name.
pub fn clean_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let s = s.trim().to_string();
    if s.is_empty() { "Program".into() } else { s }
}

/// Builds `source` for release and writes the product (and `.dmg`) into `out_dir`. `lcl` is
/// `Some` for a GUI program (a `.lpr` using the LCL). Nothing is written next to the source:
/// the compiler works in a scratch folder.
#[allow(clippy::too_many_arguments)]
pub fn export(
    source: &Path,
    mode: Mode,
    lcl: Option<&Lcl>,
    arch: Arch,
    out_dir: &Path,
    name: &str,
    info: &BundleInfo,
    dmg: bool,
    total: Option<usize>,
    on: &mut dyn FnMut(Progress),
) -> std::io::Result<Exported> {
    info.check().map_err(std::io::Error::other)?;
    let name = clean_name(name);
    std::fs::create_dir_all(out_dir)?;
    let work = std::env::temp_dir().join(format!(
        "pascoda-export-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&work)?;
    let result = run(
        source, mode, lcl, arch, out_dir, &name, info, dmg, total, &work, on,
    );
    let _ = std::fs::remove_dir_all(&work);
    result
}

#[allow(clippy::too_many_arguments)]
fn run(
    source: &Path,
    mode: Mode,
    lcl: Option<&Lcl>,
    arch: Arch,
    out_dir: &Path,
    name: &str,
    info: &BundleInfo,
    dmg: bool,
    total: Option<usize>,
    work: &Path,
    on: &mut dyn FnMut(Progress),
) -> std::io::Result<Exported> {
    let targets = arch.targets();
    // The bar: each target gets an equal share of the first 90%.
    let share = 0.9 / targets.len() as f32;
    let mut done = 0;
    let mut built: Vec<PathBuf> = Vec::new();
    let stem = source.file_stem().unwrap_or_default();
    for (i, &target) in targets.iter().enumerate() {
        let (from, to) = (share * i as f32, share * (i + 1) as f32);
        let unit_dir = work.join("units").join(target.dir());
        let exe_dir = work.join(target.dir());
        std::fs::create_dir_all(&unit_dir)?;
        std::fs::create_dir_all(&exe_dir)?;
        let sub = |p: Progress| Progress {
            label: if targets.len() > 1 {
                format!("{} ({})", p.label, target.label())
            } else {
                p.label
            },
            fraction: p.fraction.map(|f| from + f * (to - from)),
            done: p.done,
        };
        let lcl = lcl.map(|l| l.for_target(target));
        // The first export on a machine compiles the LCL for this target: 70% of its share.
        let mut lo = 0.0;
        if let Some(lcl) = &lcl
            && lcl.needs_build()
        {
            lo = 0.7;
            let warm = lcl.ensure_built_with(
                Plan { total: Some(lcl::LCL_UNITS), lo: 0.0, hi: lo },
                &mut |p| on(sub(p)),
            )?;
            if !warm.success {
                return Ok(Exported { build: warm, product: None, dmg: None });
            }
        }
        let mut extra: Vec<OsString> = match &lcl {
            Some(l) => l.project_args(&unit_dir),
            None => vec![format!("-FU{}", unit_dir.display()).into()],
        };
        // The oldest macOS (the LCL's own -WM comes earlier; the later one wins).
        if !target.is_windows() {
            extra.push(format!("-WM{}", info.macos_min()).into());
        }
        extra.push(format!("-FE{}", exe_dir.display()).into());
        let mut compiled = source.to_path_buf();
        if target.is_windows() {
            // A Windows program gets a manifest, the version information and the icon through a resource file
            // that a copy of the program source pulls in; the project's own folder is searched for everything else.
            let dir = source.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
            for flag in ["-Fu", "-Fi", "-Fo"] {
                extra.push(format!("{flag}{}", dir.display()).into());
            }
            // The project's subfolders too: a unit in one of them finds the units beside it.
            extra.extend(crate::source_folder_args(dir));
            if let Ok(text) = std::fs::read_to_string(source)
                && let Some(patched) = winres::with_directives(&text, &winres::directive_lines("pascoda.res", info.windows_min()))
            {
                let icon = match &info.icon {
                    Some(i) => Some(winres::icon_png(i, &exe_dir)?),
                    None => None,
                };
                let version = winres::Version {
                    name,
                    version: if info.version.trim().is_empty() { "1.0" } else { info.version.trim() },
                };
                std::fs::write(
                    exe_dir.join("pascoda.res"),
                    winres::res_file(&winres::manifest(name), icon.as_deref(), &version),
                )?;
                compiled = exe_dir.join(source.file_name().unwrap_or_default());
                std::fs::write(&compiled, patched)?;
            }
        }
        let plan = Plan { total, lo, hi: 1.0 };
        // The units compiled so far ride along in every later message: the app stores the last one
        // to size the bar of the next export.
        let build = build_streaming_for(target, &compiled, mode, &extra, plan, true, &mut |p| {
            done = p.done;
            on(sub(p));
        })?;
        if !build.success {
            return Ok(Exported { build, product: None, dmg: None });
        }
        built.push(if target.is_windows() {
            exe_dir.join(format!("{}.exe", stem.to_string_lossy()))
        } else {
            exe_dir.join(stem)
        });
        if i + 1 == targets.len() {
            // Only the last build's text is returned; the earlier ones had no errors.
            let kind = if target.is_windows() { Kind::Windows } else if lcl.is_some() { Kind::MacApp } else { Kind::MacTool };
            return finish(build, built, stem, kind, out_dir, name, info, dmg, work, done, on);
        }
    }
    unreachable!("an architecture always has a target")
}

/// What is being packaged.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A macOS app bundle.
    MacApp,
    /// A macOS command-line program.
    MacTool,
    /// A Windows `.exe`.
    Windows,
}

/// Joins the builds (one per CPU) and packages the result.
#[allow(clippy::too_many_arguments)]
fn finish(
    build: BuildResult,
    built: Vec<PathBuf>,
    stem: &std::ffi::OsStr,
    kind: Kind,
    out_dir: &Path,
    name: &str,
    info: &BundleInfo,
    dmg: bool,
    work: &Path,
    done: usize,
    on: &mut dyn FnMut(Progress),
) -> std::io::Result<Exported> {
    let exe = if let [only] = built.as_slice() {
        only.clone()
    } else {
        on(Progress { label: "Joining Apple Silicon and Intel".into(), fraction: Some(0.91), done });
        let joined = work.join(stem);
        lipo_create(&built, &joined)?;
        joined
    };
    on(Progress {
        label: "Packaging".into(),
        fraction: Some(0.93),
        done,
    });
    if kind == Kind::Windows {
        let to = out_dir.join(format!("{name}.exe"));
        let _ = std::fs::remove_file(&to);
        std::fs::copy(&exe, &to)?;
        let zip = if dmg {
            on(Progress { label: "Creating the zip".into(), fraction: Some(0.96), done });
            Some(make_zip(&to, name, out_dir)?)
        } else {
            None
        };
        return Ok(Exported { build, product: Some(to), dmg: zip });
    }
    let is_gui = kind == Kind::MacApp;
    let product = if is_gui {
        lcl::bundle(&exe, name, out_dir, false, info)?
    } else {
        let to = out_dir.join(name);
        let _ = std::fs::remove_file(&to);
        std::fs::copy(&exe, &to)?;
        to
    };
    let dmg_path = if dmg {
        on(Progress {
            label: "Creating the disk image".into(),
            fraction: Some(0.96),
            done,
        });
        Some(make_dmg(&product, name, out_dir, is_gui)?)
    } else {
        None
    };
    Ok(Exported {
        build,
        product: Some(product),
        dmg: dmg_path,
    })
}

/// `<out_dir>/<name>.zip` holding `<name>/<name>.exe`, as a Windows user expects to unpack it.
fn make_zip(exe: &Path, name: &str, out_dir: &Path) -> std::io::Result<PathBuf> {
    let stage = std::env::temp_dir().join(format!(
        "pascoda-zip-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let content = stage.join(name);
    std::fs::create_dir_all(&content)?;
    let result = (|| {
        std::fs::copy(exe, content.join(exe.file_name().unwrap_or_default()))?;
        let zip = out_dir.join(format!("{name}.zip"));
        let _ = std::fs::remove_file(&zip);
        // No resource forks or extended attributes: they show up as __MACOSX folders on Windows.
        let out = Command::new("ditto")
            .args(["-c", "-k", "--norsrc", "--noextattr", "--keepParent"])
            .arg(&content)
            .arg(&zip)
            .output()?;
        if !out.status.success() {
            return Err(std::io::Error::other(format!(
                "ditto failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(zip)
    })();
    let _ = std::fs::remove_dir_all(&stage);
    result
}

/// One file holding the code of every input (`lipo -create`), for a universal program.
fn lipo_create(inputs: &[PathBuf], output: &Path) -> std::io::Result<()> {
    let out = Command::new("lipo").arg("-create").args(inputs).arg("-output").arg(output).output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "lipo failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}

/// `<out_dir>/<name>.dmg` holding `product` (and an Applications shortcut for an app).
fn make_dmg(product: &Path, name: &str, out_dir: &Path, is_app: bool) -> std::io::Result<PathBuf> {
    let stage = std::env::temp_dir().join(format!(
        "pascoda-dmg-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let content = stage.join(name);
    std::fs::create_dir_all(&content)?;
    let result = (|| {
        // ditto keeps signatures, symlinks and extended attributes.
        let target = content.join(product.file_name().unwrap_or_default());
        let st = Command::new("ditto").arg(product).arg(&target).output()?;
        if !st.status.success() {
            return Err(std::io::Error::other("ditto failed"));
        }
        if is_app {
            std::os::unix::fs::symlink("/Applications", content.join("Applications"))?;
        }
        let dmg = out_dir.join(format!("{name}.dmg"));
        let _ = std::fs::remove_file(&dmg);
        let out = Command::new("diskutil")
            .args([
                "image",
                "create",
                "from",
                "--format",
                "ULFO",
                "--volumeName",
                name,
            ])
            .arg(&content)
            .arg(&dmg)
            .output()?;
        if !out.status.success() {
            return Err(std::io::Error::other(format!(
                "diskutil failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(dmg)
    })();
    let _ = std::fs::remove_dir_all(&stage);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_made_safe() {
        assert_eq!(clean_name("My App"), "My App");
        assert_eq!(clean_name("a/b:c"), "a_b_c");
        assert_eq!(clean_name("  "), "Program");
    }
}
