#!/usr/bin/env bash
# Builds Free Pascal from source into <dest>: the arm64 compiler and, to export Intel and universal
# apps, the x86_64 cross compiler, each with the unit packages Pascoda ships.
#
# Usage: packaging/fpc-build.sh [dest] [source]
#   dest    default vendor/fpc
#   source  default vendor/fpcsrc (needs compiler, rtl, packages and utils; see the README for the clone)
#
# Needs an installed Free Pascal to start from (`brew install fpc`, or FPC_BOOTSTRAP=/path/to/fpc), the
# Command Line Tools or Xcode, and about 3 GB of disk and 5 minutes. The source is copied to a scratch
# folder, so vendor/fpcsrc stays clean (KEEP_WORK=1 keeps the scratch folder).
#
# Result (the layout pascoda_fpc::bundled reads; relocatable, there is no fpc.cfg):
#   bin/fpc                                    the arm64 compiler (ppca64)
#   bin/ppcrossx64                             the cross compiler for x86_64 (an arm64 program)
#   lib/fpc/<ver>/units/aarch64-darwin/<pkg>   units for arm64
#   lib/fpc/<ver>/units/x86_64-darwin/<pkg>    units for Intel
#   lib/fpc/<ver>/units/x86_64-win64/<pkg>     units for Windows (the Intel cross compiler lists Win64 as a
#                                              target; FPC's own assembler and linker make the .exe)
#   licenses/  .pascoda-fpc (the version)
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
dest="${1:-$root/vendor/fpc}"
src="${2:-$root/vendor/fpcsrc}"

for d in compiler rtl packages utils; do
    [ -d "$src/$d" ] || { echo "fpc-build.sh: $src/$d is missing (clone the FPC sources with compiler, rtl, packages, utils)" >&2; exit 1; }
done
boot="${FPC_BOOTSTRAP:-$(command -v fpc || true)}"
[ -x "$boot" ] || { echo "fpc-build.sh: no Free Pascal to start from (brew install fpc, or set FPC_BOOTSTRAP)" >&2; exit 1; }
command -v lipo >/dev/null || { echo "fpc-build.sh: the Command Line Tools are missing (xcode-select --install)" >&2; exit 1; }

# The linker and SDK. The Command Line Tools first: their ld accepts -ld_classic, which the LCL needs.
clt=/Library/Developer/CommandLineTools
if [ -d "$clt/SDKs/MacOSX.sdk" ] && [ -x "$clt/usr/bin/ld" ]; then
    sdk="$clt/SDKs/MacOSX.sdk"; ldbin="$clt/usr/bin"
else
    sdk="$(xcrun --show-sdk-path)"; ldbin="$(dirname "$(xcrun -f ld)")"
fi
opt="-XR$sdk -FD$ldbin -ap"

# "rtl" is the rtl folder; every other name is a folder in packages/.
packages=(
    rtl rtl-objpas rtl-unicode rtl-extra rtl-console rtl-generics
    fcl-base fcl-image fcl-json fcl-process fcl-xml fcl-net fcl-stl fcl-web fcl-extra
    hash iconvenc regexpr openssl unzip zlib
    fcl-db sqlite fcl-fpcunit
    paszlib pasjpeg chm univint cocoaint
)

# Windows: the same names, minus what has no Windows build (univint, cocoaint are macOS headers, iconvenc)
# plus the Windows units the LCL needs. Packages that need fcl-web are not built for Windows (see below).
win_packages=(
    rtl rtl-objpas rtl-unicode rtl-extra rtl-console rtl-generics
    fcl-base fcl-image fcl-json fcl-process fcl-xml fcl-net fcl-stl
    hash regexpr unzip zlib
    fcl-db sqlite fcl-fpcunit
    paszlib pasjpeg chm winunits-base winunits-jedi fcl-registry
)

work="$(mktemp -d "${TMPDIR:-/tmp}/pascoda-fpc-build.XXXXXX")"
[ -n "${KEEP_WORK:-}" ] || trap 'rm -rf "$work"' EXIT
echo "fpc-build.sh: building in $work"
# Only what the pinned tag contains. A checkout can hold leftovers: compiled units that other tools wrote next
# to the sources (fpmake then skips those units: "Can't find unit URIParser") and old untracked sources (a
# stale fcl-web/src/base/fpjwt.pp shadows the real unit). Files of the sparse checkout that are not on disk
# are skipped.
if git -C "$src" rev-parse --git-dir >/dev/null 2>&1; then
    git -C "$src" ls-files -z | while IFS= read -r -d '' f; do [ -e "$src/$f" ] && printf '%s\0' "$f"; done \
        | rsync -a --from0 --files-from=- "$src/" "$work/src/"
else
    rsync -a --exclude .git "$src/" "$work/src/"
    find "$work/src" -name '*.ppu' -delete
fi
cd "$work/src"
jobs="$(sysctl -n hw.ncpu)"

# 1. The arm64 compiler, built twice by the bootstrap compiler (make cycle) so it is its own product.
make compiler_cycle OS_TARGET=darwin CPU_TARGET=aarch64 FPC="$boot" OPT="$opt" >"$work/compiler.log" 2>&1 \
    || { tail -20 "$work/compiler.log"; echo "fpc-build.sh: the compiler did not build (log: $work/compiler.log)" >&2; KEEP_WORK=1; trap - EXIT; exit 1; }
cp compiler/ppca64 "$work/ppca64"
ver="$("$work/ppca64" -iV)"
echo "fpc-build.sh: native compiler $ver ($("$work/ppca64" -iW))"

# 2. The cross compiler for Intel, with its RTL (built by compiler cycle). This comes before the arm64 RTL:
# the cycle also rebuilds the arm64 RTL with its intermediate compilers, which would leave the packages
# built against a different system.ppu ("checksum changed") if they were compiled first.
# FPC_SOFT_FPUX80: an arm64 host has no 80-bit floats, so the compiler must write x87 constants in software
# (without it the Intel RTL stops with internal error 2014051001).
make -C compiler cycle PP="$work/ppca64" FPC="$work/ppca64" OS_TARGET=darwin CPU_TARGET=x86_64 \
    OPT="$opt -dFPC_SOFT_FPUX80" >"$work/cross.log" 2>&1 \
    || { tail -20 "$work/cross.log"; echo "fpc-build.sh: the x86_64 cross compiler did not build" >&2; exit 1; }
[ -x compiler/ppcrossx64 ] || { echo "fpc-build.sh: compiler/ppcrossx64 was not produced" >&2; exit 1; }
cp compiler/ppcrossx64 "$work/ppcrossx64"
# 3. The arm64 runtime library and packages. The RTL left by the cycle belongs to the intermediate compilers
# (a "whole program optimization feedback input" mismatch), so it is cleaned and rebuilt first.
make -C rtl clean OS_TARGET=darwin CPU_TARGET=aarch64 PP="$work/ppca64" >/dev/null 2>&1 || true
make -C rtl all PP="$work/ppca64" OS_TARGET=darwin CPU_TARGET=aarch64 OPT="$opt" >"$work/rtl.log" 2>&1 \
    || { tail -20 "$work/rtl.log"; echo "fpc-build.sh: the arm64 RTL did not build" >&2; exit 1; }
make packages_all PP="$work/ppca64" FPC="$work/ppca64" OS_TARGET=darwin CPU_TARGET=aarch64 OPT="$opt" >"$work/packages.log" 2>&1 \
    || { tail -20 "$work/packages.log"; echo "fpc-build.sh: the arm64 packages did not build" >&2; exit 1; }

# And the Intel packages.
make packages_all PP="$work/ppcrossx64" FPC="$work/ppcrossx64" OS_TARGET=darwin CPU_TARGET=x86_64 OPT="$opt" \
    >"$work/packages-x64.log" 2>&1 \
    || { tail -20 "$work/packages-x64.log"; echo "fpc-build.sh: the x86_64 packages did not build" >&2; exit 1; }

# And Windows, with the same cross compiler. fcl-web does not compile for win64 in this release
# (fpjwasha256: "Identifier not found TJWTSigner"), and these packages need it: they are left out of fpmake's
# list in the scratch copy. None of them is needed for a program or the LCL.
make -C rtl all PP="$work/ppcrossx64" OS_TARGET=win64 CPU_TARGET=x86_64 OPT="$opt" >"$work/rtl-win64.log" 2>&1 \
    || { tail -20 "$work/rtl-win64.log"; echo "fpc-build.sh: the Windows RTL did not build" >&2; exit 1; }
for skip in fcl-web fppkg googleapi odata fcl-report vcl-compat; do
    sed -i '' "/IncludeTrailingPathDelimiter('$skip')/ s|^  add_|  //add_|" packages/fpmake_add.inc
done
rm -f packages/fpmake packages/fpmake.o
make packages_all PP="$work/ppcrossx64" FPC="$work/ppcrossx64" OS_TARGET=win64 CPU_TARGET=x86_64 OPT="$opt" \
    >"$work/packages-win64.log" 2>&1 \
    || { tail -20 "$work/packages-win64.log"; echo "fpc-build.sh: the Windows packages did not build" >&2; exit 1; }

# 4. Check the result: a unit of a package must load against the RTL, for both CPUs.
printf 'program t;\nuses fpimage, sysutils;\nbegin\nend.\n' > "$work/t.pas"
for t in aarch64-darwin:ppca64 x86_64-darwin:ppcrossx64 x86_64-win64:ppcrossx64; do
    triple="${t%%:*}"; ppc="$work/${t##*:}"; cpu="${triple%%-*}"; os="${triple#*-}"
    "$ppc" -T"$os" -P"$cpu" -Fu"$work/src/rtl/units/$triple" -Fu"$work/src/packages/fcl-image/units/$triple" \
        -Fu"$work/src/packages/rtl-objpas/units/$triple" -FE"$work" -Cn "$work/t.pas" >"$work/check-$triple.log" 2>&1 \
        || { tail -8 "$work/check-$triple.log"; echo "fpc-build.sh: the $triple units do not fit together" >&2; exit 1; }
done

# 5. Install into <dest>.
rm -rf "$dest"
mkdir -p "$dest/bin" "$dest/licenses"
cp "$work/ppca64" "$dest/bin/fpc"
cp "$work/ppcrossx64" "$dest/bin/ppcrossx64"
for t in aarch64-darwin x86_64-darwin x86_64-win64; do
    out="$dest/lib/fpc/$ver/units/$t"
    mkdir -p "$out"
    if [ "$t" = x86_64-win64 ]; then list=("${win_packages[@]}"); else list=("${packages[@]}"); fi
    for p in "${list[@]}"; do
        if [ "$p" = rtl ]; then from="rtl/units/$t"; else from="packages/$p/units/$t"; fi
        [ -d "$from" ] && [ -n "$(ls -A "$from")" ] || { echo "fpc-build.sh: no $t units for $p ($from)" >&2; exit 1; }
        mkdir -p "$out/$p"
        rsync -a "$from/" "$out/$p/"
    done
done

# Licences: the compiler is GPL, the runtime library and packages LGPL with a linking exception.
for f in "$root/vendor/lazarus/COPYING.GPL.txt" "$root/vendor/lazarus/COPYING.LGPL.txt" \
         "$src/rtl/COPYING.FPC" "$src/rtl/COPYING.txt" "$src/compiler/COPYING.txt"; do
    [ -f "$f" ] && cp "$f" "$dest/licenses/"
done
tag="$(git -C "$src" describe --tags --always 2>/dev/null || echo unknown)"
cat > "$dest/licenses/SOURCE.txt" <<TXT
Free Pascal $ver (compilers bin/fpc and bin/ppcrossx64 and the unit packages in lib/) was built from the
unmodified Free Pascal source, tag $tag:

  https://gitlab.com/freepascal.org/fpc/source  (tag $tag)

It was built with packaging/fpc-build.sh of Pascoda, which adds only the compiler define FPC_SOFT_FPUX80
for the Intel cross compiler.

The compiler is licensed under the GNU General Public License (licenses/COPYING.GPL.txt). The runtime
library and the packages are licensed under the GNU Lesser General Public License with the linking
exception of licenses/COPYING.FPC, so programs you build with them keep your own licence.
TXT
printf '%s\n' "$ver" > "$dest/.pascoda-fpc"
echo "fpc-build.sh: Free Pascal $ver (arm64 + x86_64) -> $dest ($(du -sh "$dest" | cut -f1))"
