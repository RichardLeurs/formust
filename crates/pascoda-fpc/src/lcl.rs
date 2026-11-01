//! Building GUI applications against the Lazarus Component Library (LCL) with the Cocoa
//! widgetset, using plain `fpc` (no Lazarus install, no lazbuild).

use crate::{BuildResult, Mode, Plan, Progress, Target, allow_debugging};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Oldest macOS the generated apps support (`-WM` and `LSMinimumSystemVersion`).
pub const MIN_MACOS: &str = "14.0";

/// Units compiled into the cache so projects normally find them precompiled.
const WARM_UNITS: &[&str] = &[
    "Interfaces",
    "Forms",
    "Controls",
    "Graphics",
    "StdCtrls",
    "ExtCtrls",
    "Dialogs",
    "Menus",
    "ComCtrls",
    "Buttons",
    "Grids",
    "ActnList",
    "Clipbrd",
    "LCLType",
    "LCLIntf",
    "LResources",
];

#[derive(Debug, Clone)]
pub struct Lcl {
    /// Lazarus source tree (contains `lcl/`, `components/lazutils/`).
    pub lazarus_dir: PathBuf,
    /// Where precompiled LCL units live. Keyed by compiler version and Lazarus commit.
    pub cache: PathBuf,
    /// What the units are compiled for; each target has its own cache folder.
    pub target: Target,
}

impl Lcl {
    /// Looks for `vendor/lazarus` in `start` and its parents, then in `$PASCODA_HOME`, then in
    /// the `Resources` folder of the app bundle this program runs from, and last
    /// in the source tree this crate was built from. Projects and test scratch folders can live
    /// anywhere.
    pub fn find(start: &Path) -> Option<Self> {
        Self::find_in(start, None)
    }

    /// [`Lcl::find`] with the Lazarus the project asks for (`pascoda_project::load_lazarus`). Which Lazarus builds
    /// the LCL, first match: the folder the project names (when it is a Lazarus source tree); a Lazarus source tree
    /// that `start` lies in (a project of Lazarus's own tree must use that tree's LCL); the vendored one.
    pub fn find_in(start: &Path, lazarus: Option<&Path>) -> Option<Self> {
        let named = lazarus.filter(|d| is_lazarus_tree(d)).map(Path::to_path_buf);
        let enclosing = || start.ancestors().find(|d| is_lazarus_tree(d)).map(Path::to_path_buf);
        let vendored = || {
            search_dirs(start)
                .iter()
                .map(|d| d.join("vendor/lazarus"))
                .find(|d| d.join("lcl/interfaces/cocoa").is_dir())
        };
        let lazarus_dir = named.or_else(enclosing).or_else(vendored)?;
        let home = std::env::var_os("HOME")?;
        let key = format!("{}-{}", fpc_version(), commit(&lazarus_dir));
        Some(Self {
            cache: PathBuf::from(home)
                .join("Library/Caches/Pascoda/lcl")
                .join(key),
            lazarus_dir,
            target: Target::native(),
        })
    }

    /// The same LCL for another target: its own cache folder (units differ per CPU), the same sources.
    pub fn for_target(&self, target: Target) -> Lcl {
        let mut name = self.cache.file_name().unwrap_or_default().to_string_lossy().into_owned();
        name.truncate(name.strip_suffix(self.target.cache_suffix()).map_or(name.len(), str::len));
        name.push_str(target.cache_suffix());
        Lcl {
            lazarus_dir: self.lazarus_dir.clone(),
            cache: self.cache.with_file_name(name),
            target,
        }
    }

    fn source_dirs(&self) -> Vec<PathBuf> {
        source_dirs_for(&self.lazarus_dir, self.target)
    }

    /// Whether the LCL's sources have a unit called `name` (`LCLType` for `lcltype.pp`; the case is ignored).
    pub fn provides_unit(&self, name: &str) -> bool {
        self.source_dirs().iter().any(|dir| {
            std::fs::read_dir(dir).into_iter().flatten().flatten().any(|e| {
                let p = e.path();
                p.extension().is_some_and(|x| x.eq_ignore_ascii_case("pp") || x.eq_ignore_ascii_case("pas"))
                    && p.file_stem().is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case(name))
            })
        })
    }

    /// Arguments every LCL compile shares, for the cache and for projects alike.
    fn common_args(&self) -> Vec<OsString> {
        let mut args: Vec<OsString> = defines_for(&self.lazarus_dir, self.target)
            .into_iter()
            .map(Into::into)
            .collect();
        if self.target.is_windows() {
            // A window program, not a console one; FPC links it with its own linker.
            args.push("-WG".into());
            return args;
        }
        args.extend([
            format!("-WM{MIN_MACOS}").into(),
            // fpc 3.2 emits Objective-C method lists that Apple's current linker rejects;
            // the classic linker accepts them. Apple has deprecated that flag.
            "-k-ld_classic".into(),
            "-k-framework".into(),
            "-kUserNotifications".into(),
        ]);
        args
    }

    fn ready_marker(&self) -> PathBuf {
        self.cache.join(".ready")
    }

    /// Compiles the LCL into the cache once; later calls return immediately.
    pub fn ensure_built(&self) -> std::io::Result<BuildResult> {
        self.ensure_built_with(Plan::WHOLE, &mut |_| {})
    }

    /// Whether the LCL still has to be compiled (the first GUI build on a machine).
    pub fn needs_build(&self) -> bool {
        !self.ready_marker().exists()
    }

    /// [`Lcl::ensure_built`] with progress, filling `plan`'s share of the bar.
    pub fn ensure_built_with(
        &self,
        plan: Plan,
        on: &mut dyn FnMut(Progress),
    ) -> std::io::Result<BuildResult> {
        if self.ready_marker().exists() {
            return Ok(BuildResult {
                success: true,
                diagnostics: vec![],
                raw_output: String::new(),
            });
        }
        std::fs::create_dir_all(&self.cache)?;
        let warm = self.cache.join("warm.lpr");
        std::fs::write(
            &warm,
            format!(
                "program warm;\n{{$mode objfpc}}{{$H+}}\nuses {};\nbegin\nend.\n",
                WARM_UNITS.join(", ")
            ),
        )?;
        let mut args = self.common_args();
        args.push("-Cn".into()); // units only, no link
        args.push(format!("-FU{}", self.cache.display()).into());
        for d in self.source_dirs() {
            args.push(format!("-Fu{}", d.display()).into());
        }
        let result = crate::build_streaming_for(self.target, &warm, Mode::ObjFpc, &args, plan, false, &mut |mut p| {
            p.label = p.label.replace("Compiling", "Preparing LCL:");
            on(p);
        })?;
        if result.success {
            std::fs::write(self.ready_marker(), b"")?;
        }
        Ok(result)
    }

    /// Search paths for a project: cached units first, sources as the fallback.
    pub(crate) fn project_args(&self, units_out: &Path) -> Vec<OsString> {
        let mut args = self.common_args();
        args.push(format!("-FU{}", units_out.display()).into());
        args.push(format!("-Fu{}", self.cache.display()).into());
        for d in self.source_dirs() {
            args.push(format!("-Fu{}", d.display()).into());
        }
        args
    }
}

/// The LCL's source folders for macOS, relative to the Lazarus tree.
fn source_dirs(lazarus: &Path) -> Vec<PathBuf> {
    source_dirs_for(lazarus, Target::MacArm64)
}

/// The LCL's source folders for `target`: the widgetset differs (Cocoa on macOS, Win32 on Windows).
fn source_dirs_for(lazarus: &Path, target: Target) -> Vec<PathBuf> {
    let widgetset = if target.is_windows() { "lcl/interfaces/win32" } else { "lcl/interfaces/cocoa" };
    let mut dirs = vec!["lcl"];
    if !target.is_windows() {
        dirs.push("lcl/nonwin32");
    }
    // packager/registration: LazarusPackageIntf, which the main unit of every Lazarus package uses.
    dirs.extend(["lcl/forms", "lcl/widgetset", widgetset, "components/lazutils", "components/freetype", "packager/registration"]);
    dirs.iter().map(|p| lazarus.join(p)).collect()
}

/// The defines and include folder the LCL is compiled with, for macOS.
fn defines(lazarus: &Path) -> Vec<String> {
    defines_for(lazarus, Target::MacArm64)
}

fn defines_for(lazarus: &Path, target: Target) -> Vec<String> {
    let mut defines = vec!["-dLCL".to_string(), if target.is_windows() { "-dLCLwin32" } else { "-dLCLcocoa" }.to_string()];
    // The Cocoa widgetset of older Lazaruses refuses to compile without COCOA ("wrong interfaces source");
    // the newer ones do not look at it.
    if !target.is_windows() {
        defines.push("-dCOCOA".into());
    }
    defines.push(format!("-Fi{}", lazarus.join("lcl/include").display()));
    defines
}

/// Compiler switches that let a tool parse the LCL's sources as a project would (pasls).
/// Without them the LCL's `{$I lcl_defines.inc}` is not found and no LCL type resolves.
pub fn pasls_options(lazarus: &Path) -> Vec<String> {
    let mut options = defines(lazarus);
    options.extend(
        source_dirs(lazarus)
            .iter()
            .map(|d| format!("-Fu{}", d.display())),
    );
    options
}

/// Units in the LCL cache of this machine's build of it; the first GUI build compiles about
/// this many, which sizes its progress bar.
pub const LCL_UNITS: usize = 160;

/// Builds a GUI program (a `.lpr` using the LCL) and returns the plain executable's result.
pub fn build_gui(source: &Path, mode: Mode, lcl: &Lcl) -> std::io::Result<BuildResult> {
    build_gui_progress(source, mode, lcl, None, &mut |_| {})
}

/// [`build_gui`] with progress. `total` is the number of units the last build of this program
/// compiled. The first build on a machine also compiles the LCL, which takes the first 70% of the bar.
pub fn build_gui_progress(
    source: &Path,
    mode: Mode,
    lcl: &Lcl,
    total: Option<usize>,
    on: &mut dyn FnMut(Progress),
) -> std::io::Result<BuildResult> {
    build_gui_progress_with(source, mode, lcl, &[], total, on)
}

/// [`build_gui_progress`] with extra compiler arguments, such as a project file's search paths.
pub fn build_gui_progress_with(
    source: &Path,
    mode: Mode,
    lcl: &Lcl,
    extra: &[OsString],
    total: Option<usize>,
    on: &mut dyn FnMut(Progress),
) -> std::io::Result<BuildResult> {
    let lo = if lcl.needs_build() { 0.7 } else { 0.0 };
    if lo > 0.0 {
        let warm_plan = Plan {
            total: Some(LCL_UNITS),
            lo: 0.0,
            hi: lo,
        };
        let warm = lcl.ensure_built_with(warm_plan, on)?;
        if !warm.success {
            return Ok(warm);
        }
    }
    let dir = source
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let units_out = dir.join("lib");
    std::fs::create_dir_all(&units_out)?;
    let units_out = units_out.canonicalize()?;
    let plan = Plan { total, lo, hi: 1.0 };
    let mut args = lcl.project_args(&units_out);
    args.extend(extra.iter().cloned());
    crate::build_streaming_for(lcl.target, source, mode, &args, plan, false, on)
}

/// Wraps `exe` into `<dir>/<name>.app` so macOS treats it as a real application (Dock icon,
/// menu bar, light/dark appearance). Returns the bundle path.
pub fn make_bundle(exe: &Path, name: &str) -> std::io::Result<PathBuf> {
    let dir = exe.parent().unwrap_or(Path::new("."));
    bundle(exe, name, dir, true, &BundleInfo::default())
}

/// What an exported app says about itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BundleInfo {
    /// `com.example.app`; `com.pascoda.<name>` when empty.
    pub bundle_id: String,
    /// `1.0` when empty.
    pub version: String,
    /// A `.icns` (copied) or `.png` (converted with `sips`) file.
    pub icon: Option<PathBuf>,
    /// The oldest macOS the app runs on, as `-WM` and `LSMinimumSystemVersion` say it (`14.0`); `MIN_MACOS` when empty.
    pub min_macos: String,
    /// The oldest Windows, as `{$SETPEOSVERSION}` and `{$SETPESUBSYSVERSION}` say it (`6.1` is Windows 7,
    /// `10.0` Windows 10). Empty leaves the compiler's own.
    pub min_windows: String,
}

/// Whether `v` is a version like `14` or `14.0`.
fn version_number_ok(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    (1..=2).contains(&parts.len()) && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

impl BundleInfo {
    /// The oldest macOS to build for (`MIN_MACOS` when none is given).
    pub fn macos_min(&self) -> &str {
        match self.min_macos.trim() {
            "" => MIN_MACOS,
            v => v,
        }
    }

    /// The oldest Windows to build for, when one is given.
    pub fn windows_min(&self) -> Option<&str> {
        Some(self.min_windows.trim()).filter(|v| !v.is_empty())
    }

    /// `Err` with a sentence for the user when a minimum version is not a number like `14.0`.
    pub fn check_minimums(&self) -> Result<(), String> {
        let (mac, win) = (self.min_macos.trim(), self.min_windows.trim());
        if !mac.is_empty() && !version_number_ok(mac) {
            return Err("The minimum macOS version is a number like 14.0.".into());
        }
        if !win.is_empty() && !version_number_ok(win) {
            return Err("The minimum Windows version is a number like 6.1 (Windows 7) or 10.0 (Windows 10).".into());
        }
        Ok(())
    }

    /// `Err` with a sentence for the user when a field cannot be used.
    pub fn check(&self) -> Result<(), String> {
        self.check_minimums()?;
        let id = self.bundle_id.trim();
        if !id.is_empty()
            && (!id.contains('.')
                || id.starts_with('.')
                || id.ends_with('.')
                || !id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'))
        {
            return Err(
                "The bundle identifier uses letters, digits, '-' and dots, like com.example.myapp."
                    .into(),
            );
        }
        let v = self.version.trim();
        if !v.is_empty() && !v.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return Err("The version uses digits and dots, like 1.0 or 2.1.3.".into());
        }
        if let Some(i) = &self.icon {
            let ext = i
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase);
            if !i.is_file() || !matches!(ext.as_deref(), Some("icns" | "png")) {
                return Err("The icon must be an existing .icns or .png file.".into());
            }
        }
        Ok(())
    }
}

/// `<dir>/<name>.app` around `exe`. `debuggable` re-signs for lldb (a development build); a
/// release bundle gets a plain ad-hoc signature instead.
pub(crate) fn bundle(
    exe: &Path,
    name: &str,
    dir: &Path,
    debuggable: bool,
    info: &BundleInfo,
) -> std::io::Result<PathBuf> {
    let bundle = dir.join(format!("{name}.app"));
    let macos = bundle.join("Contents/MacOS");
    if bundle.exists() {
        std::fs::remove_dir_all(&bundle)?;
    }
    std::fs::create_dir_all(&macos)?;
    std::fs::copy(exe, macos.join(name))?;
    let icon_file = match &info.icon {
        Some(icon) => {
            let res = bundle.join("Contents/Resources");
            std::fs::create_dir_all(&res)?;
            let to = res.join("AppIcon.icns");
            if icon
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("icns"))
            {
                std::fs::copy(icon, &to)?;
            } else {
                png_to_icns(icon, &to)?;
            }
            Some("AppIcon")
        }
        None => None,
    };
    std::fs::write(
        bundle.join("Contents/Info.plist"),
        info_plist(name, info, icon_file),
    )?;
    if debuggable {
        allow_debugging(&bundle)?;
    } else {
        let st = Command::new("codesign")
            .args(["--force", "--deep", "-s", "-"])
            .arg(&bundle)
            .output()?;
        if !st.status.success() {
            return Err(std::io::Error::other(format!(
                "codesign failed: {}",
                String::from_utf8_lossy(&st.stderr).trim()
            )));
        }
    }
    Ok(bundle)
}

/// Makes an `.icns` from a PNG: every size of an iconset with `sips`, then `iconutil`.
fn png_to_icns(png: &Path, out: &Path) -> std::io::Result<()> {
    let set = std::env::temp_dir().join(format!("pascoda-icon-{}.iconset", std::process::id()));
    let _ = std::fs::remove_dir_all(&set);
    std::fs::create_dir_all(&set)?;
    let result = (|| {
        for (px, name) in [
            (16, "icon_16x16"),
            (32, "icon_16x16@2x"),
            (32, "icon_32x32"),
            (64, "icon_32x32@2x"),
            (128, "icon_128x128"),
            (256, "icon_128x128@2x"),
            (256, "icon_256x256"),
            (512, "icon_256x256@2x"),
            (512, "icon_512x512"),
            (1024, "icon_512x512@2x"),
        ] {
            let st = Command::new("sips")
                .args(["-z", &px.to_string(), &px.to_string()])
                .arg(png)
                .arg("--out")
                .arg(set.join(format!("{name}.png")))
                .output()?;
            if !st.status.success() {
                return Err(std::io::Error::other(format!(
                    "could not read the icon: {}",
                    String::from_utf8_lossy(&st.stderr).trim()
                )));
            }
        }
        let st = Command::new("iconutil")
            .args(["-c", "icns"])
            .arg(&set)
            .arg("-o")
            .arg(out)
            .output()?;
        if !st.status.success() {
            return Err(std::io::Error::other(format!(
                "could not build the icon: {}",
                String::from_utf8_lossy(&st.stderr).trim()
            )));
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&set);
    result
}

/// No `NSPrincipalClass`: LCL creates its own `NSApplication` subclass, and if AppKit makes a
/// plain one first the app dies with `unrecognized selector lclSetLCLMainLoop:`.
fn info_plist(name: &str, info: &BundleInfo, icon: Option<&str>) -> String {
    let esc = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let id = match info.bundle_id.trim() {
        "" => format!("com.pascoda.{}", name.replace(' ', "-")),
        id => id.to_string(),
    };
    let version = match info.version.trim() {
        "" => "1.0",
        v => v,
    };
    let icon = icon
        .map(|i| format!("<key>CFBundleIconFile</key><string>{i}</string>\n"))
        .unwrap_or_default();
    let (id, version, name) = (esc(&id), esc(version), esc(name));
    let min = esc(info.macos_min());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>{name}</string>
<key>CFBundleIdentifier</key><string>{id}</string>
<key>CFBundleName</key><string>{name}</string>
{icon}<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>{version}</string>
<key>CFBundleVersion</key><string>{version}</string>
<key>LSMinimumSystemVersion</key><string>{min}</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
"#
    )
}

fn fpc_version() -> String {
    Command::new(crate::locate_fpc().unwrap_or_else(|| "fpc".into()))
        .arg("-iV")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

/// Folders that may hold `vendor/`, best first. See `Lcl::find`.
pub fn search_dirs(start: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = start.ancestors().map(Path::to_path_buf).collect();
    dirs.extend(fixed_dirs());
    dirs
}

/// The folders that may hold `vendor/` whatever file is open: `$PASCODA_HOME`, the app bundle, the
/// user's `Application Support/Pascoda`, and last the source tree this was built from (so a release
/// build run from a checkout finds `vendor/` too; a path that does not exist on another Mac is just skipped).
pub fn fixed_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    dirs.extend(std::env::var_os("PASCODA_HOME").map(PathBuf::from));
    // Contents/MacOS/<exe> -> Contents/Resources
    if let Ok(exe) = std::env::current_exe()
        && let Some(contents) = exe.parent().and_then(Path::parent)
    {
        dirs.push(contents.join("Resources"));
    }
    // Where the tools are installed when the bundle does not have them (see `vendor::install_command`).
    dirs.extend(crate::vendor::user_root());
    dirs.extend(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .map(Path::to_path_buf),
    );
    dirs
}

/// Whether `dir` is a Lazarus source tree: it has the LCL (with the Cocoa widgetset) and LazUtils.
pub fn is_lazarus_tree(dir: &Path) -> bool {
    dir.join("lcl/interfaces/cocoa").is_dir() && dir.join("components/lazutils").is_dir()
}

fn commit(lazarus_dir: &Path) -> String {
    // A bundled copy has no .git; bundle.sh records the commit it was copied from.
    if let Ok(c) = std::fs::read_to_string(lazarus_dir.join(".pascoda-commit")) {
        let c = c.trim();
        if !c.is_empty() {
            return c.into();
        }
    }
    match commit_from_git(lazarus_dir).as_str() {
        // A tree without git history (a download): the folder tells it apart from other trees.
        "unknown" => {
            let canon = lazarus_dir.canonicalize().unwrap_or_else(|_| lazarus_dir.to_path_buf());
            let mut h: u64 = 0xcbf29ce484222325;
            for b in canon.to_string_lossy().bytes() {
                h = (h ^ u64::from(b)).wrapping_mul(0x100000001b3);
            }
            format!("tree-{:08x}", h as u32)
        }
        git => git.to_string(),
    }
}

fn commit_from_git(lazarus_dir: &Path) -> String {
    Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(lazarus_dir)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

#[cfg(test)]
mod minimum_version_tests {
    use super::*;

    #[test]
    fn a_minimum_is_a_number_with_one_or_two_parts_and_blank_is_the_default() {
        let with = |mac: &str, win: &str| BundleInfo { min_macos: mac.into(), min_windows: win.into(), ..Default::default() };
        assert!(with("", "").check_minimums().is_ok());
        assert!(with("14", "").check_minimums().is_ok());
        assert!(with("13.0", "6.1").check_minimums().is_ok());
        assert!(with("14.0.1", "").check_minimums().is_err(), "three parts");
        assert!(with("macOS 14", "").check_minimums().is_err());
        assert!(with("", "10.").check_minimums().is_err(), "an empty part");
        assert!(with("", "Windows 7").check_minimums().is_err());
        assert_eq!(with("", "").macos_min(), MIN_MACOS, "the default macOS");
        assert_eq!(with("13.0", "").macos_min(), "13.0");
        assert_eq!(with("", "").windows_min(), None);
        assert_eq!(with("", " 10.0 ").windows_min(), Some("10.0"));
    }

    #[test]
    fn the_bundle_says_the_chosen_minimum_macos() {
        let info = BundleInfo { min_macos: "13.0".into(), ..Default::default() };
        let plist = info_plist("Notes", &info, None);
        assert!(plist.contains("<key>LSMinimumSystemVersion</key><string>13.0</string>"), "{plist}");
        let default = info_plist("Notes", &BundleInfo::default(), None);
        assert!(default.contains(&format!("<string>{MIN_MACOS}</string>")), "{default}");
    }
}
