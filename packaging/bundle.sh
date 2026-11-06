#!/usr/bin/env bash
# Builds Formust for each CPU: one app and one disk image per build, each with its own name.
#
#   Formust-aarch64-darwin.app / .dmg    Apple Silicon only
#   Formust-x86_64-darwin.app / .dmg     Intel only
#   Formust-universal-darwin.app / .dmg  one app for both CPUs (the two executables joined with lipo)
#
# Usage: packaging/bundle.sh [output-dir]      (default: ~/Downloads/Formust)
#   ARCHES="aarch64-darwin universal-darwin" packaging/bundle.sh   builds only the listed ones (default: all three)
#   NO_DMG=1                                  skips the disk images
#   A bundle is replaced only when that same build is made again. Nothing else in the output folder is deleted,
#   so the three builds (and any earlier Formust.app) stay side by side.
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
out="${1:-$HOME/Downloads/Formust}"
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
    log="$(mktemp "${TMPDIR:-/tmp}/formust-quiet.XXXXXX")"
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
say "Bundling the Formust IDE app"
say ""
for a in $ARCHES; do
    case "$a" in
        aarch64-darwin|x86_64-darwin|universal-darwin) ;;
        *) fail "unknown build '$a' in ARCHES (use aarch64-darwin, x86_64-darwin, universal-darwin)" ;;
    esac
done
wanted() { case " $ARCHES " in *" $1 "*) return 0 ;; esac; return 1; }

# The number of steps, for the progress bar: two steps before the bundles (reading Cargo.toml, building), then per
# bundle 2 steps (the copy of the build and the signing), minus the disk image with NO_DMG, plus the notarization
# with NOTARY_PROFILE. With all three bundles and the defaults this is 11.
total=2
for b in aarch64-darwin x86_64-darwin universal-darwin; do
    wanted "$b" || continue
    n=2
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
step "Reading Cargo.toml"

# Single source of truth for version and copyright.
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/crates/formust-app/Cargo.toml" | head -1)"
# The IDE's identity: [workspace.metadata.formust] in the root Cargo.toml (name, bundle_id, copyright, min_macos).
meta() { awk -v k="$1" '
    /^[[:space:]]*#/ { next }
    /^\[/ { in_table = ($0 == "[workspace.metadata.formust]"); next }
    in_table && $1 == k && $2 == "=" { sub(/^[^"]*"/, ""); sub(/".*$/, ""); print; exit }' "$root/Cargo.toml"; }
app_name="$(meta name)"
bundle_id="$(meta bundle_id)"
copyright="$(meta copyright)"
# The oldest macOS the IDE runs on: `min_macos` under [workspace.metadata.formust] in the root Cargo.toml.
ide_min="$(meta min_macos)"
[ -n "$version" ] && [ -n "$app_name" ] && [ -n "$bundle_id" ] && [ -n "$copyright" ] && [ -n "$ide_min" ] \
    || fail "cannot read the version, name, bundle_id, copyright or min_macos (Cargo.toml [workspace.metadata.formust])"
# Rust and the linker record this as the binary's minimum (LC_BUILD_VERSION).
export MACOSX_DEPLOYMENT_TARGET="$ide_min"
if [ -n "${NOTARY_PROFILE:-}" ] && [ -z "${CODESIGN_IDENTITY:-}" ]; then
    fail "NOTARY_PROFILE needs CODESIGN_IDENTITY"
fi

# The release builds: Apple Silicon for the arm64 bundle, Intel for the x86_64 one, both for the universal.
step "Building the app"
# Cargo's output goes to a log, shown only if the build fails.
cargo_log="$(mktemp "${TMPDIR:-/tmp}/formust-cargo.XXXXXX")"
if wanted aarch64-darwin || wanted universal-darwin; then
    (cd "$root" && cargo build --release -p formust-app) >"$cargo_log" 2>&1 \
        || { cat "$cargo_log" >&2; fail "cargo build failed (output above)"; }
fi
if wanted x86_64-darwin || wanted universal-darwin; then
    (cd "$root" && cargo build --release -p formust-app --target x86_64-apple-darwin) >"$cargo_log" 2>&1 \
        || { cat "$cargo_log" >&2; fail "cargo build for Intel failed (output above)"; }
fi
rm -f "$cargo_log"
# The build directory is set in .cargo/config.toml, so ask cargo where it is.
target="$(cd "$root" && cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
bin="$target/release/formust"
bin_intel="$target/x86_64-apple-darwin/release/formust"
if wanted aarch64-darwin || wanted universal-darwin; then
    [ -x "$bin" ] || fail "$bin not found"
fi
if wanted x86_64-darwin || wanted universal-darwin; then
    [ -x "$bin_intel" ] || fail "$bin_intel not found"
fi

# Makes one bundle: <name> is aarch64-darwin, x86_64-darwin or universal-darwin.
make_bundle() {
    local name="$1" app dmg zip cpus=() exe archs stage
    app="$out/Formust-$name.app"
    dmg="$out/Formust-$name.dmg"
    zip="$out/Formust-$name.zip"
    case "$name" in
        aarch64-darwin) cpus=(aarch64) ;;
        x86_64-darwin) cpus=(x86_64) ;;
        universal-darwin) cpus=(aarch64 x86_64) ;;
    esac

    # This bundle's own app folder; a rebuild of the same build replaces it, nothing else is touched.
    rm -rf "$app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    exe="$app/Contents/MacOS/formust"
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
    cp "$root/packaging/Formust.icns" "$app/Contents/Resources/Formust.icns"

    # NSPrincipalClass is deliberately absent: eframe/winit create their own NSApplication.
    # The bundle identifier is a placeholder until the real developer account is used (Cargo.toml bundle_id).
    cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
<key>CFBundleName</key><string>$app_name</string>
<key>CFBundleDisplayName</key><string>$app_name</string>
<key>CFBundleIdentifier</key><string>$bundle_id</string>
<key>CFBundleExecutable</key><string>formust</string>
<key>CFBundleIconFile</key><string>Formust</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>$version</string>
<key>CFBundleShortVersionString</key><string>$version</string>
<key>LSMinimumSystemVersion</key><string>$ide_min</string>
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
        stage="$(mktemp -d "${TMPDIR:-/tmp}/formust-dmg.XXXXXX")"
        cp -R "$app" "$stage/"
        ln -s /Applications "$stage/Applications"
        rm -f "$dmg"
        hdiutil create -quiet -volname "Formust $name" -srcfolder "$stage" -ov -format UDZO "$dmg"
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