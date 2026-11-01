#!/usr/bin/env bash
# Builds Pascoda for each CPU: one app and one disk image per build, each with its own name.
#
#   Pascoda-aarch64-darwin.app / .dmg    Apple Silicon only (vendor/fpc: the arm64 compiler, the Intel cross
#                                        compiler and the Windows units)
#   Pascoda-x86_64-darwin.app / .dmg     Intel only (vendor/fpc-intel: the compiler that runs on Intel, from
#                                        packaging/fpc-intel-build.sh)
#   Pascoda-universal-darwin.app / .dmg  one app for both CPUs (the two executables joined with lipo; both
#                                        compilers and both pasls binaries)
#
# Usage: packaging/bundle.sh [output-dir]      (default: ~/Downloads/Pascoda)
#   ARCHES="aarch64-darwin universal-darwin" packaging/bundle.sh   builds only the listed ones (default: all three)
#   NO_DMG=1                                  skips the disk images
#   A bundle is replaced only when that same build is made again. Nothing else in the output folder is deleted,
#   so the three builds (and any earlier Pascoda.app) stay side by side.
#   The terminal is cleared at the start. A progress bar shows the step count and what each step is doing; the
#   script's own messages are green when the output is a terminal (the tools it runs print their usual output).
#
# Signing:
#   default                  ad-hoc signature (`codesign -s -`): runs on this Mac only.
#   CODESIGN_IDENTITY=<id>   sign with that identity (e.g. "Developer ID Application: Name (TEAM)"),
#                            hardened runtime + secure timestamp, as notarization requires.
#   NOTARY_PROFILE=<name>    additionally zip the app, submit it with
#                            `xcrun notarytool submit --wait --keychain-profile <name>`
#                            (create the profile once with `xcrun notarytool store-credentials`)
#                            and staple the ticket with `xcrun stapler staple`.
#                            Requires CODESIGN_IDENTITY; this path has not been exercised.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="${1:-$HOME/Downloads/Pascoda}"
ARCHES="${ARCHES:-aarch64-darwin x86_64-darwin universal-darwin}"

# Output: green on a terminal, plain when piped. Errors go to stderr and end the script.
if [ -t 1 ]; then
    clear
    green=$'\033[32m'
    red=$'\033[31m'
    reset=$'\033[0m'
else
    green=""
    red=""
    reset=""
fi
# On a terminal the progress bar is redrawn on one line; this ends that line before other output.
bar_open=0
end_bar() { if [ "$bar_open" = 1 ]; then printf '\n'; bar_open=0; fi; }
say() { end_bar; printf '%s%s%s\n' "$green" "$*" "$reset"; }
fail() { end_bar; printf '%sbundle.sh: %s%s\n' "$red" "$*" "$reset" >&2; exit 1; }
# Runs a command with its output kept in a log, shown only if it fails.
quiet() {
    local log
    log="$(mktemp "${TMPDIR:-/tmp}/pascoda-quiet.XXXXXX")"
    if ! "$@" >"$log" 2>&1; then
        end_bar; cat "$log" >&2; rm -f "$log"
        fail "'$1' failed (output above)"
    fi
    rm -f "$log"
}

say ""
while IFS= read -r line; do say "$line"; done <<'ART'
 ____     _      ____     ____    ___    ____      _
|  _ \   / \    / ___|   / ___|  / _ \  |  _ \    / \
| |_) | / _ \   \___ \  | |     | | | | | | | |  / _ \
|  __/ / ___ \   ___) | | |___  | |_| | | |_| | / ___ \
|_|   /_/   \_\ |____/   \____|  \___/  |____/ /_/   \_\
ART
say ""
say "Bundling the Pascoda IDE app"
say ""
for a in $ARCHES; do
    case "$a" in
        aarch64-darwin|x86_64-darwin|universal-darwin) ;;
        *) fail "unknown build '$a' in ARCHES (use aarch64-darwin, x86_64-darwin, universal-darwin)" ;;
    esac
done
wanted() { case " $ARCHES " in *" $1 "*) return 0 ;; esac; return 1; }

# The number of steps, for the progress bar: two steps before the bundles (checks, build), then per bundle
# 6 steps (7 for universal, which has two compiler copies and the lipo join), minus the disk image with NO_DMG,
# plus the notarization with NOTARY_PROFILE. With all three bundles and the defaults this is 21.
total=2
for b in aarch64-darwin x86_64-darwin universal-darwin; do
    wanted "$b" || continue
    n=6
    [ "$b" = universal-darwin ] && n=7
    [ -n "${NO_DMG:-}" ] && n=$((n - 1))
    [ -n "${NOTARY_PROFILE:-}" ] && n=$((n + 1))
    total=$((total + n))
done

# A progress bar line before each step: the bar fills as the steps complete, then the step's message.
step_no=0
# The bar for `done` of `total` steps: full when all are done.
bar_for() {
    local done="$1" width=30 filled bar="" i
    filled=$(( done * width / total ))
    for ((i = 0; i < width; i++)); do
        if [ "$i" -lt "$filled" ]; then bar+="█"; else bar+="░"; fi
    done
    printf '%s' "$bar"
}
step() {
    step_no=$((step_no + 1))
    local text="[$(bar_for "$step_no")] $step_no/$total  $1"
    if [ -t 1 ]; then
        # Redraw on the same line: back to column 0, clear the line, keep it within the window's width.
        local cols
        cols="$(stty size </dev/tty 2>/dev/null | cut -d' ' -f2)"
        [ "${cols:-0}" -gt 20 ] 2>/dev/null || cols=80
        printf '\r\033[K%s%s%s' "$green" "${text:0:$((cols - 1))}" "$reset"
        bar_open=1
    else
        say "$text"
    fi
}
step "Checking vendor/ and the compilers"

# Single source of truth for version and copyright.
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/crates/pascoda-app/Cargo.toml" | head -1)"
copyright="$(sed -n 's/^pub const COPYRIGHT: &str = "\(.*\)";/\1/p' "$root/crates/pascoda-app/src/about.rs")"
min_macos="$(sed -n 's/^pub const MIN_MACOS: &str = "\(.*\)";/\1/p' "$root/crates/pascoda-fpc/src/lcl.rs")"
[ -n "$version" ] && [ -n "$copyright" ] && [ -n "$min_macos" ] || fail "cannot read version/copyright/min macOS"
if [ -n "${NOTARY_PROFILE:-}" ] && [ -z "${CODESIGN_IDENTITY:-}" ]; then
    fail "NOTARY_PROFILE needs CODESIGN_IDENTITY"
fi

# What a bundle needs under a folder that holds lazarus/, fpcsrc/ and pasls/ (the same list as
# pascoda_fpc::vendor::Part): the shared sources and the pasls of each CPU it runs on.
needed() { # <folder> <what> <aarch64|x86_64>...
    local folder="$1" what="$2" n cpu
    shift 2
    local list=(lazarus/lcl/interfaces/cocoa fpcsrc/rtl fpcsrc/packages/fcl-db/src fpcsrc/packages/fcl-fpcunit/src)
    for cpu in "$@"; do list+=("pasls/dist/$cpu-darwin/pasls"); done
    for n in "${list[@]}"; do
        [ -e "$folder/$n" ] || fail "$n is missing from $what (set up vendor/ first, see README)"
    done
}
# The compilers are checked before the long build. Apple Silicon: vendor/fpc (packaging/fpc-build.sh).
# Intel: vendor/fpc-intel (packaging/fpc-intel-build.sh).
fpcver="$(cat "$root/vendor/fpc/.pascoda-fpc" 2>/dev/null || true)"
require() { [ -e "$1" ] || fail "$2"; }
if wanted aarch64-darwin || wanted universal-darwin; then
    [ -n "$fpcver" ] || fail "vendor/fpc has no version marker (run packaging/fpc-build.sh)"
    needed "$root/vendor" "vendor/" aarch64
    for n in fpc/bin/fpc fpc/bin/ppcrossx64 "fpc/lib/fpc/$fpcver/units/aarch64-darwin/rtl" \
             "fpc/lib/fpc/$fpcver/units/x86_64-darwin/rtl" "fpc/lib/fpc/$fpcver/units/x86_64-win64/rtl"; do
        require "$root/vendor/$n" "vendor/$n is missing (run packaging/fpc-build.sh)"
    done
fi
if wanted x86_64-darwin || wanted universal-darwin; then
    [ -n "$fpcver" ] || fail "vendor/fpc has no version marker (run packaging/fpc-build.sh)"
    needed "$root/vendor" "vendor/" x86_64
    for n in bin/fpc "lib/fpc/$fpcver/units/x86_64-darwin/rtl"; do
        require "$root/vendor/fpc-intel/$n" "vendor/fpc-intel/$n is missing (run packaging/fpc-intel-build.sh)"
    done
fi

# The release builds: Apple Silicon for the arm64 bundle, Intel for the x86_64 one, both for the universal.
step "Building the app"
# Cargo's output goes to a log, shown only if the build fails.
cargo_log="$(mktemp "${TMPDIR:-/tmp}/pascoda-cargo.XXXXXX")"
if wanted aarch64-darwin || wanted universal-darwin; then
    (cd "$root" && cargo build --release -p pascoda-app) >"$cargo_log" 2>&1 \
        || { cat "$cargo_log" >&2; fail "cargo build failed (output above)"; }
fi
if wanted x86_64-darwin || wanted universal-darwin; then
    (cd "$root" && cargo build --release -p pascoda-app --target x86_64-apple-darwin) >"$cargo_log" 2>&1 \
        || { cat "$cargo_log" >&2; fail "cargo build for Intel failed (output above)"; }
fi
rm -f "$cargo_log"
# The build directory is set in .cargo/config.toml, so ask cargo where it is.
target="$(cd "$root" && cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
bin="$target/release/pascoda-app"
bin_intel="$target/x86_64-apple-darwin/release/pascoda-app"
if wanted aarch64-darwin || wanted universal-darwin; then
    [ -x "$bin" ] || fail "$bin not found"
fi
if wanted x86_64-darwin || wanted universal-darwin; then
    [ -x "$bin_intel" ] || fail "$bin_intel not found"
fi

# Makes one bundle: <name> is aarch64-darwin, x86_64-darwin or universal-darwin.
make_bundle() {
    local name="$1" app dmg zip cpus=() exe res lz archs stage
    app="$out/Pascoda-$name.app"
    dmg="$out/Pascoda-$name.dmg"
    zip="$out/Pascoda-$name.zip"
    case "$name" in
        aarch64-darwin) cpus=(aarch64) ;;
        x86_64-darwin) cpus=(x86_64) ;;
        universal-darwin) cpus=(aarch64 x86_64) ;;
    esac

    # This bundle's own app folder; a rebuild of the same build replaces it, nothing else is touched.
    rm -rf "$app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    exe="$app/Contents/MacOS/pascoda-app"
    case "$name" in
        aarch64-darwin)
            step "$name: copying the Apple Silicon build into the bundle"
            cp "$bin" "$exe" ;;
        x86_64-darwin)
            step "$name: copying the Intel build into the bundle"
            cp "$bin_intel" "$exe" ;;
        universal-darwin)
            step "$name: joining the two builds into one executable (lipo)"
            lipo -create "$bin" "$bin_intel" -output "$exe" ;;
    esac
    # The executable must be the CPUs this bundle is for (checked silently; the result is not printed).
    archs="$(lipo -archs "$exe" 2>/dev/null || true)"
    case "$name" in
        aarch64-darwin) want="arm64" ;;
        x86_64-darwin) want="x86_64" ;;
        universal-darwin) want="arm64 x86_64" ;;
    esac
    for a in $want; do
        case " $archs " in *" $a "*) ;; *) fail "$exe has architectures '$archs', expected $want" ;; esac
    done
    cp "$root/packaging/Pascoda.icns" "$app/Contents/Resources/Pascoda.icns"

    # Toolchain sources the app needs on a Mac without this checkout: the LCL (runs the forms), CodeTools and
    # the FPC sources (completion, builds), the pasls binary of each CPU, and the compilers of those CPUs.
    res="$app/Contents/Resources/vendor"
    lz="$root/vendor/lazarus"
    mkdir -p "$res/lazarus/components" "$res/fpcsrc"
    for cpu in "${cpus[@]}"; do mkdir -p "$res/pasls/dist/$cpu-darwin"; done
    cp "$lz"/COPYING*.txt "$res/lazarus/"
    step "$name: copying the LCL, CodeTools and the FPC sources"
    rsync -a --exclude '.git' "$lz/lcl" "$res/lazarus/"
    for c in lazutils codetools freetype; do rsync -a --exclude '.git' "$lz/components/$c" "$res/lazarus/components/"; done
    git -C "$lz" rev-parse --short HEAD > "$res/lazarus/.pascoda-commit" 2>/dev/null || echo bundled > "$res/lazarus/.pascoda-commit"
    # The licence texts stay, though fpcsrc.exclude drops *.txt (the first matching rule wins).
    rsync -a --include='COPYING*' --exclude '.git' --exclude-from="$root/packaging/fpcsrc.exclude" "$root/vendor/fpcsrc/" "$res/fpcsrc/"
    for cpu in "${cpus[@]}"; do
        cp "$root/vendor/pasls/dist/$cpu-darwin/pasls" "$res/pasls/dist/$cpu-darwin/"
    done
    cp "$root/vendor/pasls/COPYING" "$res/pasls/COPYING"
    # The revisions of what was copied, next to the Lazarus one above.
    git -C "$root/vendor/fpcsrc" describe --tags --always > "$res/fpcsrc/.pascoda-commit" 2>/dev/null || echo bundled > "$res/fpcsrc/.pascoda-commit"
    git -C "$root/vendor/pasls" rev-parse --short HEAD > "$res/pasls/.pascoda-commit" 2>/dev/null || echo bundled > "$res/pasls/.pascoda-commit"

    # The compilers: the Apple Silicon folder (arm64 compiler, Intel cross compiler, Windows units) for the
    # arm64 and universal bundles, the Intel folder for the Intel and universal bundles.
    case "$name" in aarch64-darwin|universal-darwin)
        step "$name: copying the Apple Silicon compilers and their units (the largest part: several minutes)"
        rsync -a "$root/vendor/fpc/" "$res/fpc/" ;;
    esac
    case "$name" in x86_64-darwin|universal-darwin)
        step "$name: copying the Intel compiler and its units"
        rsync -a "$root/vendor/fpc-intel/" "$res/fpc-intel/" ;;
    esac
    case "$name" in aarch64-darwin|universal-darwin)
        for n in fpc/bin/fpc fpc/bin/ppcrossx64 fpc/.pascoda-fpc fpc/licenses/SOURCE.txt \
                 "fpc/lib/fpc/$fpcver/units/aarch64-darwin/rtl" "fpc/lib/fpc/$fpcver/units/x86_64-darwin/rtl" \
                 "fpc/lib/fpc/$fpcver/units/x86_64-win64/rtl"; do
            [ -e "$res/$n" ] || fail "$n is missing from the bundle"
        done
        ;;
    esac
    case "$name" in x86_64-darwin|universal-darwin)
        [ -x "$res/fpc-intel/bin/fpc" ] || fail "the Intel compiler is missing from the bundle"
        ;;
    esac
    needed "$res" "the bundle" "${cpus[@]}"

    # Components that ship with Pascoda (the VLC package; File > Import Component Package... offers it).
    mkdir -p "$app/Contents/Resources/components"
    step "$name: copying the components"
    rsync -a --exclude '.git' "$root/components/" "$app/Contents/Resources/components/"

    # NSPrincipalClass is deliberately absent: eframe/winit create their own NSApplication.
    # CFBundleIdentifier is a PLACEHOLDER; replace it with the identifier of the real developer account.
    cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
<key>CFBundleName</key><string>Pascoda</string>
<key>CFBundleDisplayName</key><string>Pascoda</string>
<key>CFBundleIdentifier</key><string>com.richardleurs.pascoda</string>
<key>CFBundleExecutable</key><string>pascoda-app</string>
<key>CFBundleIconFile</key><string>Pascoda</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>$version</string>
<key>CFBundleShortVersionString</key><string>$version</string>
<key>LSMinimumSystemVersion</key><string>$min_macos</string>
<key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
<key>NSHighResolutionCapable</key><true/>
<key>NSDisabledDictationMenuItem</key><true/>
<key>NSDisabledCharacterPaletteMenuItem</key><true/>
<key>NSSupportsAutomaticGraphicsSwitching</key><true/>
<key>NSHumanReadableCopyright</key><string>$copyright</string>
</dict>
</plist>
PLIST
    plutil -lint "$app/Contents/Info.plist" >/dev/null

    step "$name: signing the app"
    if [ -n "${CODESIGN_IDENTITY:-}" ]; then
        quiet codesign --force --deep --options runtime --timestamp -s "$CODESIGN_IDENTITY" "$app"
    else
        quiet codesign --force --deep -s - "$app"
    fi
    quiet codesign --verify --deep --strict "$app"

    if [ -n "${NOTARY_PROFILE:-}" ]; then
        step "$name: notarizing the app (Apple's service: this waits, sometimes for many minutes)"
        rm -f "$zip"
        ditto -c -k --keepParent "$app" "$zip"
        xcrun notarytool submit "$zip" --wait --keychain-profile "$NOTARY_PROFILE"
        xcrun stapler staple "$app"
        rm -f "$zip"
    fi
    if [ -z "${NO_DMG:-}" ]; then
        step "$name: making the disk image (the app and an Applications link)"
        stage="$(mktemp -d "${TMPDIR:-/tmp}/pascoda-dmg.XXXXXX")"
        cp -R "$app" "$stage/"
        ln -s /Applications "$stage/Applications"
        rm -f "$dmg"
        hdiutil create -quiet -volname "Pascoda $name" -srcfolder "$stage" -ov -format UDZO "$dmg"
        rm -rf "$stage"
    fi
    return 0
}

for name in aarch64-darwin x86_64-darwin universal-darwin; do
    if wanted "$name"; then make_bundle "$name"; fi
done

say ""
say "[$(bar_for "$step_no")] $step_no/$total  Done ($(date +%H:%M:%S))" 
say ""