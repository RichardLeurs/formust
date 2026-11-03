#!/usr/bin/env bash
# Builds the Free Pascal compiler that runs on Intel Macs, with its own units, into <dest>
# (default vendor/fpc-intel). Pascoda.app on an Intel Mac uses it to compile programs for Intel:
# the arm64 bundle's cross compiler (bin/ppcrossx64) is an arm64 program and cannot run there.
#
# Usage: packaging/fpc-intel-build.sh [dest] [source]
#   dest    default vendor/fpc-intel
#   source  default vendor/fpcsrc (the same sources as fpc-build.sh)
#
# Needs vendor/fpc from packaging/fpc-build.sh (its bin/ppcrossx64 builds this compiler), the Command Line
# Tools or Xcode, and about 2 GB of disk and 10 minutes. The result runs on x86_64 only (Rosetta 2 runs it on
# Apple Silicon for testing; that is not what the Intel bundle is for).
#
# Result (the layout pascoda_fpc::bundled reads; relocatable, there is no fpc.cfg):
#   bin/fpc                                    the Intel compiler (ppcx64): an x86_64 program that writes x86_64 code
#   lib/fpc/<ver>/units/x86_64-darwin/<pkg>    units built by that compiler (not the cross compiler's: the two
#                                              compilers' units do not load into each other)
#   licenses/  .pascoda-fpc (the version)
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
dest="${1:-$root/vendor/fpc-intel}"
src="${2:-$root/vendor/fpcsrc}"
cross="$root/vendor/fpc/bin/ppcrossx64"

for d in compiler rtl packages utils; do
    [ -d "$src/$d" ] || { echo "fpc-intel-build.sh: $src/$d is missing (clone the FPC sources, see the README)" >&2; exit 1; }
done
[ -x "$cross" ] || { echo "fpc-intel-build.sh: $cross is missing (run packaging/fpc-build.sh first)" >&2; exit 1; }
command -v lipo >/dev/null || { echo "fpc-intel-build.sh: the Command Line Tools are missing (xcode-select --install)" >&2; exit 1; }

# The linker and SDK, as fpc-build.sh chooses them.
clt=/Library/Developer/CommandLineTools
if [ -d "$clt/SDKs/MacOSX.sdk" ] && [ -x "$clt/usr/bin/ld" ]; then
    sdk="$clt/SDKs/MacOSX.sdk"; ldbin="$clt/usr/bin"
else
    sdk="$(xcrun --show-sdk-path)"; ldbin="$(dirname "$(xcrun -f ld)")"
fi
opt="-XR$sdk -FD$ldbin -ap"

# The same packages as the Intel units of fpc-build.sh ("rtl" is the rtl folder, the others are in packages/).
packages=(
    rtl rtl-objpas rtl-unicode rtl-extra rtl-console rtl-generics
    fcl-base fcl-image fcl-json fcl-process fcl-xml fcl-net fcl-stl fcl-web fcl-extra
    hash iconvenc regexpr openssl unzip zlib
    fcl-db sqlite fcl-fpcunit
    paszlib pasjpeg chm univint cocoaint
)

work="$(mktemp -d "${TMPDIR:-/tmp}/pascoda-fpc-intel.XXXXXX")"
[ -n "${KEEP_WORK:-}" ] || trap 'rm -rf "$work"' EXIT
echo "fpc-intel-build.sh: building in $work"
# Only the files the pinned tag tracks (see fpc-build.sh for why a checkout's leftovers are not copied).
if git -C "$src" rev-parse --git-dir >/dev/null 2>&1; then
    git -C "$src" ls-files -z | while IFS= read -r -d '' f; do [ -e "$src/$f" ] && printf '%s\0' "$f"; done \
        | rsync -a --from0 --files-from=- "$src/" "$work/src/"
else
    rsync -a --exclude .git "$src/" "$work/src/"
    find "$work/src" -name '*.ppu' -delete
fi
cd "$work/src"

# 1. The compiler for an x86_64 host, built by the cross compiler: its code is x86_64 (CPU_SOURCE) and it
# writes x86_64 code (CPU_TARGET). FPC_SOFT_FPUX80 is the same define the cross compiler was built with.
make -C compiler cycle PP="$cross" FPC="$cross" OS_TARGET=darwin CPU_TARGET=x86_64 CPU_SOURCE=x86_64 \
    OPT="$opt -dFPC_SOFT_FPUX80 -Fl$sdk/usr/lib" >"$work/compiler.log" 2>&1 \
    || { tail -20 "$work/compiler.log"; echo "fpc-intel-build.sh: the Intel compiler did not build (log: $work/compiler.log)" >&2; KEEP_WORK=1; trap - EXIT; exit 1; }
[ -x compiler/ppcx64 ] || { echo "fpc-intel-build.sh: compiler/ppcx64 was not produced" >&2; exit 1; }
[ "$(file -b compiler/ppcx64 | grep -c 'x86_64')" -eq 1 ] || { echo "fpc-intel-build.sh: compiler/ppcx64 is not an x86_64 program" >&2; exit 1; }
cp compiler/ppcx64 "$work/ppcx64"
ver="$("$work/ppcx64" -iV)"
echo "fpc-intel-build.sh: Intel compiler $ver"

# 2. Its own RTL and packages. The RTL left by the cycle belongs to the intermediate compilers, so it is
# cleaned and rebuilt first (as for the arm64 bundle).
make -C rtl clean OS_TARGET=darwin CPU_TARGET=x86_64 PP="$work/ppcx64" >/dev/null 2>&1 || true
make -C rtl all PP="$work/ppcx64" OS_TARGET=darwin CPU_TARGET=x86_64 OPT="$opt" >"$work/rtl.log" 2>&1 \
    || { tail -20 "$work/rtl.log"; echo "fpc-intel-build.sh: the RTL did not build" >&2; exit 1; }
make packages_all PP="$work/ppcx64" FPC="$work/ppcx64" OS_TARGET=darwin CPU_TARGET=x86_64 OPT="$opt" \
    >"$work/packages.log" 2>&1 \
    || { tail -20 "$work/packages.log"; echo "fpc-intel-build.sh: the packages did not build" >&2; exit 1; }

# 3. Check: a unit of a package must load against this RTL, and the program must run.
printf 'program t;\nuses fpimage, sysutils;\nbegin\nend.\n' > "$work/t.pas"
"$work/ppcx64" -Tdarwin -Px86_64 -Fu"$work/src/rtl/units/x86_64-darwin" -Fu"$work/src/packages/fcl-image/units/x86_64-darwin" \
    -Fu"$work/src/packages/rtl-objpas/units/x86_64-darwin" -FE"$work" -Cn "$work/t.pas" >"$work/check.log" 2>&1 \
    || { tail -8 "$work/check.log"; echo "fpc-intel-build.sh: the Intel units do not fit together" >&2; exit 1; }

# 4. Install.
rm -rf "$dest"
mkdir -p "$dest/bin" "$dest/licenses"
cp "$work/ppcx64" "$dest/bin/fpc"
out="$dest/lib/fpc/$ver/units/x86_64-darwin"
mkdir -p "$out"
for p in "${packages[@]}"; do
    if [ "$p" = rtl ]; then from="rtl/units/x86_64-darwin"; else from="packages/$p/units/x86_64-darwin"; fi
    [ -d "$from" ] && [ -n "$(ls -A "$from")" ] || { echo "fpc-intel-build.sh: no units for $p ($from)" >&2; exit 1; }
    mkdir -p "$out/$p"
    rsync -a "$from/" "$out/$p/"
done

# fpcres: the resource compiler the compiler runs for {$R file.res} on macOS. The compiler looks for it in
# -FD and then on PATH; Pascoda puts the compiler's own folder first, so it ships here next to the compiler.
mkdir -p "$work/fpcres-units"
"$dest/bin/fpc" -Mobjfpc -O2 -Xs -Fu"$work/src/packages/fcl-res/src" -Fu"$work/src/utils/fpcres" \
    -FU"$work/fpcres-units" -o"$dest/bin/fpcres" "$work/src/utils/fpcres/fpcres.pas" >"$work/fpcres.log" 2>&1 \
    || { tail -20 "$work/fpcres.log"; echo "fpc-intel-build.sh: fpcres did not build (log: $work/fpcres.log)" >&2; exit 1; }
[ -x "$dest/bin/fpcres" ] || { echo "fpc-intel-build.sh: bin/fpcres was not produced" >&2; exit 1; }

for f in "$root/vendor/lazarus/COPYING.GPL.txt" "$root/vendor/lazarus/COPYING.LGPL.txt" \
         "$src/rtl/COPYING.FPC" "$src/rtl/COPYING.txt" "$src/compiler/COPYING.txt"; do
    [ -f "$f" ] && cp "$f" "$dest/licenses/"
done
printf '%s\n' "$ver" > "$dest/.pascoda-fpc"
echo "fpc-intel-build.sh: Free Pascal $ver (x86_64 host) -> $dest ($(du -sh "$dest" | cut -f1))"
