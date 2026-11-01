//! What a program is built for. One compiler run makes one target; a universal macOS app is two
//! runs joined by `lipo` (see `export`). A Windows target would join this enum.

use std::ffi::OsString;
use std::path::PathBuf;

/// One operating system and CPU the compiler can produce code for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Target {
    /// Apple Silicon macOS: the compiler that ships with Pascoda, `bin/fpc`.
    #[default]
    MacArm64,
    /// Intel macOS: the cross compiler `bin/ppcrossx64` (an arm64 program that writes x86_64 code).
    MacX86_64,
    /// Windows 64-bit: the same cross compiler (it lists Win64 among its targets); FPC's own assembler
    /// and linker make the `.exe`, so no Windows tools are needed.
    WinX64,
}

/// The target a Mac of this CPU (`std::env::consts::ARCH`) builds for natively: Intel Macs compile for Intel,
/// Apple Silicon for Apple Silicon.
pub fn native_for(arch: &str) -> Target {
    if arch == "x86_64" { Target::MacX86_64 } else { Target::MacArm64 }
}

impl Target {
    /// The target of this Mac's own CPU. Its compiler is the bundled one for that CPU (`bin/fpc`).
    pub fn native() -> Target {
        native_for(std::env::consts::ARCH)
    }

    /// Folder name for this target's build output (`arm64`, `x86_64`).
    pub fn dir(self) -> &'static str {
        match self {
            Target::MacArm64 => "arm64",
            Target::MacX86_64 => "x86_64",
            Target::WinX64 => "win64",
        }
    }

    /// FPC's name for the target (`<cpu>-<os>`): the folder of its units.
    pub fn triple(self) -> &'static str {
        match self {
            Target::MacArm64 => "aarch64-darwin",
            Target::MacX86_64 => "x86_64-darwin",
            Target::WinX64 => "x86_64-win64",
        }
    }

    pub fn is_windows(self) -> bool {
        self == Target::WinX64
    }

    /// The word shown to the user.
    pub fn label(self) -> &'static str {
        match self {
            Target::MacArm64 => "Apple Silicon",
            Target::MacX86_64 => "Intel",
            Target::WinX64 => "Windows",
        }
    }

    /// Whether this is the target of this Mac's own CPU (its compiler runs here and makes the code natively).
    pub fn is_native(self) -> bool {
        self == Target::native()
    }

    /// Arguments that select the target. They must come before the configuration file, whose unit
    /// folders name `$fpctarget`. Empty for the native target, so its command line is unchanged.
    pub fn args(self) -> Vec<OsString> {
        match self {
            Target::MacArm64 => Vec::new(),
            Target::MacX86_64 => vec!["-Tdarwin".into(), "-Px86_64".into()],
            Target::WinX64 => vec!["-Twin64".into(), "-Px86_64".into()],
        }
    }

    /// The compiler that makes code for this target, if it is installed.
    /// An Intel Mac has no arm64 compiler (its bundle carries the Intel one only), so Apple Silicon is `None` there.
    pub fn compiler(self) -> Option<PathBuf> {
        match self {
            Target::MacArm64 if native_for(std::env::consts::ARCH) == Target::MacX86_64 => None,
            Target::MacArm64 => crate::locate_fpc(),
            Target::MacX86_64 | Target::WinX64 => crate::bundled::cross_fpc_for(self),
        }
    }

    /// Part of the LCL cache folder's name. Empty for the native target (its cache keeps its name).
    pub fn cache_suffix(self) -> &'static str {
        match self {
            Target::MacArm64 => "",
            Target::MacX86_64 => "-x86_64",
            Target::WinX64 => "-win64",
        }
    }
}

/// Which CPUs an exported app runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Arch {
    /// Apple Silicon only.
    #[default]
    Arm64,
    /// Apple Silicon and Intel in one file.
    Universal,
    /// Intel only: the one choice an Intel Mac can make natively.
    Intel,
    /// Windows 64-bit (an `.exe`, zipped).
    Windows,
}

impl Arch {
    /// The name stored in `pascoda.toml`.
    pub fn name(self) -> &'static str {
        match self {
            Arch::Arm64 => "arm64",
            Arch::Universal => "universal",
            Arch::Intel => "intel",
            Arch::Windows => "windows",
        }
    }

    /// The architecture stored under `name`; anything unknown is the default.
    pub fn from_name(name: &str) -> Arch {
        match name.trim() {
            "universal" => Arch::Universal,
            "intel" => Arch::Intel,
            "windows" => Arch::Windows,
            _ => Arch::Arm64,
        }
    }

    /// The targets that are compiled, in order.
    pub fn targets(self) -> &'static [Target] {
        match self {
            Arch::Arm64 => &[Target::MacArm64],
            Arch::Universal => &[Target::MacArm64, Target::MacX86_64],
            Arch::Intel => &[Target::MacX86_64],
            Arch::Windows => &[Target::WinX64],
        }
    }

    /// What the result runs on, for the user.
    pub fn label(self) -> &'static str {
        match self {
            Arch::Arm64 => "Apple Silicon",
            Arch::Universal => "Apple Silicon + Intel",
            Arch::Intel => "Apple Intel",
            Arch::Windows => "Windows 64-bit",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_native_target_adds_nothing_to_the_command_line() {
        assert!(Target::MacArm64.args().is_empty());
        assert_eq!(Target::MacX86_64.args(), ["-Tdarwin", "-Px86_64"]);
        assert_eq!(Target::WinX64.args(), ["-Twin64", "-Px86_64"]);
    }

    #[test]
    fn architectures_round_trip_and_unknown_names_are_arm64() {
        for a in [Arch::Arm64, Arch::Universal, Arch::Intel, Arch::Windows] {
            assert_eq!(Arch::from_name(a.name()), a);
        }
        assert_eq!(Arch::from_name("native"), Arch::Arm64);
        assert_eq!(Arch::from_name(""), Arch::Arm64);
        assert_eq!(Arch::Universal.targets(), [Target::MacArm64, Target::MacX86_64]);
        assert_eq!(Arch::Intel.targets(), [Target::MacX86_64]);
    }

    #[test]
    fn a_mac_builds_natively_for_its_own_cpu() {
        assert_eq!(native_for("aarch64"), Target::MacArm64);
        assert_eq!(native_for("x86_64"), Target::MacX86_64);
        assert_eq!(Target::native(), native_for(std::env::consts::ARCH));
        assert!(Target::native().is_native());
        let other = if Target::native() == Target::MacArm64 { Target::MacX86_64 } else { Target::MacArm64 };
        assert!(!other.is_native(), "the other Mac CPU is a cross target");
        assert!(!Target::WinX64.is_native());
    }

    #[test]
    fn each_target_has_its_own_lcl_cache_and_output_folder() {
        let all = [Target::MacArm64, Target::MacX86_64, Target::WinX64];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a.cache_suffix(), b.cache_suffix());
                assert_ne!(a.dir(), b.dir());
                assert_ne!(a.triple(), b.triple());
            }
        }
    }
}
