# Vendored sources

Unmodified copies from the reference mruby tree, made by `tools/vendor_compiler.sh`
on 2026-09-11. Nothing here is edited by hand: adapt in `compiler/csrc/shim.c` and
`compiler/build.rs` instead. If a patch ever becomes necessary, record the diff here.

| path | from | version | licence |
|---|---|---|---|
| `mruby-compiler/{include,src,LICENSE,README.md}` | `mrbgems/mruby-compiler` of mruby | 4.1.0-rc, commit `3cf73ee` | MIT, Copyright (c) HASUMI Hitoshi 2024 (`mruby-compiler/LICENSE`); gem author "mruby and PicoRuby developers" |
| `prism/{include,src,LICENSE.md}` | `mrbgems/mruby-compiler/lib/prism` (git submodule) | Prism 1.9.0, commit `c0e3781` | MIT, Copyright 2022-present Shopify Inc. (`prism/LICENSE.md`) |
| `prism/generated/{include,src}` | `build/prism/` of a reference build | generated from Prism 1.9.0's ERB templates by the reference `rake` | as Prism |
| `mrbconf.h` | `include/mrbconf.h` of mruby | 4.1.0-rc | MIT, mruby developers (`../../LICENSE-mruby`) |

`prism/generated/` holds the files Prism generates from `templates/*.erb`
(`include/prism/ast.h`, `include/prism/diagnostic.h`, `src/{diagnostic,node,prettyprint,serialize,token_type}.c`).
They are taken from the reference build instead of being regenerated (no Ruby needed to build
this crate). Their `#line` directives name `mrbgems/mruby-compiler/lib/prism/templates/...`,
the reference build's rewrite; they only affect diagnostics of the C compiler.

## Updating

1. In the reference tree (`../../ref/mruby`, or `MRUBY_SRC`), check out the new tag and run
   `rake` once so that `build/prism/` holds the generated sources for its Prism.
2. `tools/vendor_compiler.sh`
3. Update the table above and `sabiruby_mrc_version()` in `compiler/csrc/shim.c`.
4. `cargo test -p sabiruby-compiler` (golden tests against the reference `mrbc` output).
