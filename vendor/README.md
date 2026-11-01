### Vendored sources (in `vendor/`, not committed)

| Folder | What | Source | Used for |
|---|---|---|---|
| `vendor/fpc` | the Free Pascal compilers (`bin/fpc` for arm64, `bin/ppcrossx64` for Intel) and their precompiled units for both CPUs, with licences and a source offer | built from `vendor/fpcsrc` by `packaging/fpc-build.sh` | compiling Pascal; wins over an `fpc` on the `PATH` |
| `vendor/pasls` | Pascal language server | github.com/genericptr/pascal-language-server | completion, go-to-definition |
| `vendor/lazarus` | Lazarus source tree (tested at commit `2dbcbed8`) | gitlab.com/freepascal.org/lazarus/lazarus | `pasls` CodeTools backend and the LCL for GUI apps |
| `vendor/fpcsrc` | FPC 3.2.4-rc2 `compiler`, `rtl`, `packages` and `utils` (tag `release_3_2_4_rc2`) | gitlab.com/freepascal.org/fpc/source | builds `vendor/fpc`; lets the language server read standard units, and the database and FPCUnit sources |

Set them up from the project root. Install Free Pascal first (`brew install fpc`): it starts the build of `vendor/fpc`, which `pasls` is then built with:

```sh
mkdir -p vendor && cd vendor
git clone --depth 1 --recurse-submodules https://github.com/genericptr/pascal-language-server.git pasls
git clone --depth 1 --filter=blob:none https://gitlab.com/freepascal.org/lazarus/lazarus.git lazarus
git clone --depth 1 --branch release_3_2_4_rc2 --filter=blob:none --sparse \
  https://gitlab.com/freepascal.org/fpc/source.git fpcsrc
(cd fpcsrc && git sparse-checkout set compiler rtl packages utils)
(cd pasls/src && LAZARUSDIR="$PWD/../../lazarus" bash build_fpc.sh)
cd .. && packaging/fpc-build.sh                 # about 5 minutes: arm64 and Intel compilers into vendor/fpc
```


The language server binary ends up in `vendor/pasls/dist/aarch64-darwin/pasls`. Without it the
IDE still works; completion is simply off. Without `vendor/lazarus`, GUI programs cannot be
built. Without `vendor/fpc` the IDE uses the `fpc` on your `PATH` (or `PASCODA_FPC`), so a checkout also
works with only `brew install fpc`; `vendor/fpc` makes it use the same compiler and units as the shipped app.