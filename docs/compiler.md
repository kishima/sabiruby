# Compiler

`sabiruby run foo.rb`, `sabiruby -e CODE` and `sabiruby compile` compile Ruby source with the
**reference compiler itself**: mruby 4.1.0-rc's `mrbgems/mruby-compiler` (Prism 1.9.0 as the
parser, mruby's code generator), built as C and linked into the command. The compiler is not
the subject of this project (nor of the book), so it is not ported to Rust; what matters is
that the bytecode is exactly the reference's. The plan this follows is
[`compiler-plan.md`](compiler-plan.md).

## Layout

* **Crate `sabiruby-compiler`** (`compiler/`, a workspace member). std, links libc, does
  not depend on `sabiruby`. The VM crate stays `no_std` and does not use it; only the
  `sabiruby` binary does, through the default feature `compiler`
  (`compiler = ["std", "dep:sabiruby-compiler"]`). `tools/check_no_std.sh` builds with
  `--no-default-features` and is unaffected.
* `compiler/vendor/`: unmodified copies of `mruby-compiler`, Prism, Prism's generated sources
  and `mrbconf.h` (origin, versions, licences and the update procedure in
  [`compiler/vendor/VENDOR.md`](../compiler/vendor/VENDOR.md); `tools/vendor_compiler.sh`).
* `compiler/build.rs`: compiles them with `cc` as C99.
* `compiler/csrc/shim.c`: the only C written here (below).
* `compiler/src/ffi.rs`: three `extern "C"` functions, the crate's only `unsafe`.
* `compiler/src/lib.rs`: `compile(src, &Options) -> Result<Vec<u8>, CompileError>`,
  `Diagnostic`, `version()`.

## The standalone path, as the reference mrbc

The reference build compiles `mrbc` in a sub-build without the mruby VM
(`lib/mruby/build.rb`, `generate_mrbc_build`: `disable_libmruby`). Its compile flags for the
compiler (`build/host/mrbc/mrbgems/mruby-compiler/src/*.o.flags` in a reference build) have
`-DMRB_NO_GEMS`, so `mrbgem.rake` does not add `MRC_TARGET_MRUBY`; the `.d` files show that
`mruby.h` is never included. With neither `MRC_TARGET_MRUBY` nor `MRC_TARGET_MRUBYC`,
`include/mrc_common.h` takes the standalone path: `mrb_state` is `void`, only `mrbconf.h` is
read, and `prism_xallocator.h` allocates with libc. `build.rs` uses the same configuration:
`PRISM_XALLOCATOR`, `PRISM_DEPTH_MAXIMUM=256`, `PRISM_BUILD_MINIMAL` (a debug reference build
has `MRC_DEBUG` instead, which adds assertions, poisons freed ireps and keeps Prism's AST
printer; the golden tests compare with the `mrbc` of the Docker image), no `MRC_TARGET_*`, no `MRC_NO_STDIO`, no `MRC_INT32` (so `MRB_INT64`, like SabiRuby).

The standalone compiler still links against two mruby functions, `mrb_intern` and
`mrb_sym_name`; the reference `mrbc.c` defines dummies for them at its end, and so does the
shim.

## The shim

`sabiruby_mrc_compile(src, len, filename, flags, &out, &out_len, &diag)` does what `mrbc`'s
`main` does, from memory instead of files: `mrc_ccontext_new(NULL)`, `mrc_ccontext_filename`
(it copies the string), `no_exec`/`no_ext_ops`/`no_optimize`, `mrc_load_string_cxt` on a
NUL-terminated copy of the source, the diagnostics, `mrc_irep_remove_lv` if asked,
`mrc_dump_irep` into memory (`MRC_DUMP_DEBUG_INFO` for `-g`), and every free. Rust never
touches `mrc_ccontext` (it has bit fields), so no bindgen. Diagnostics come back as one string
(records separated by `0x1e`, fields by `0x1f`) and are split in Rust.

`mrc_presym.c` writes a static variable on every parse, so `compile` holds a global lock.

## Verification

`compiler/tests/golden.rs` compiles every `.rb` of the repository that has a `.mrb` made by the
reference `mrbc` (Docker image `kishima/mruby:4.1.0-rc`) and requires identical bytes, with the
file name the generating script passed to `mrbc` (it ends up in the DBG section under `-g`):

| set | files | options | result |
|---|---:|---|---|
| `tests/fixtures/*.rb` | 17 | none | identical |
| `tests/mrbtest/src/*.rb` (mruby's test suite and gem tests) | 61 | `-g` (DBG and LVAR) | identical |
| `bench/src/*.rb` | 7 | none | identical |

A negative control (the test files without `-g`) makes all 61 differ. Checked once by hand as
well: the ten embedded mrblib binaries (`src/mrblib*.mrb`) recompiled from their sources are
identical, and the messages and exit codes of `sabiruby compile` on syntax errors are
byte-identical to `mrbc`'s.

The verification baseline does not move: `tools/mrbtest.sh`, `tools/fixtures.sh` and
`tools/bench.sh` keep compiling with the reference `mrbc` in Docker; the golden tests show that
the embedded compiler agrees with it.

## CLI

* `sabiruby run FILE`: a file starting with `RITE` runs as before; anything else is compiled
  with `filename` = FILE and `debug_info` (DBG is not read by the VM, but the LVAR section is
  needed by `local_variables` and `Proc#parameters`, and the reference only writes it with `-g`).
* `sabiruby -e 'CODE' [args]`: file name `-e`, as `mruby -e`.
* `sabiruby compile FILE [-o OUT] [-g] [--remove-lv] [--no-ext-ops] [--no-optimize]`: `mrbc`'s
  options. The default output replaces the extension with `.mrb`; unlike `mrbc`, a name
  without an extension gets `.mrb` appended (`mrbc` would reuse the input name and overwrite
  the source). `-o -` writes to stdout.
* `sabiruby dump FILE`: `.rb` or `.mrb`.
* Errors print as `FILE:LINE:COL: message` on stderr, exit code 1 (as `mrbc`; warnings are not
  printed, as `mrbc` without `--verbose`).
* `sabiruby --version` also names the embedded compiler.
* `sabiruby mrbtest` is unchanged (`.mrb` only).

## Build cost and platforms

A clean build of the C part: about 2 s (debug, `-O0`) and 7 s (release), on one core
(`cc` compiles the 35 files, 34 vendored plus the shim, one after another). The
`sabiruby-compiler` package is 2.9 MiB (394 KiB compressed). CI builds and tests on Linux and macOS; Windows (MSVC) should work but is
not tested. **wasm32 is not supported** (the C side would need clang with a wasm sysroot, i.e.
wasi-sdk or emscripten, chosen by the build script); the VM library itself builds for
`wasm32-unknown-unknown` with `--no-default-features`.

## Publishing

`sabiruby-compiler` first, then `sabiruby` (its binary depends on it); `cargo publish
--workspace` does both in order. `sabiruby` 0.1.0 is already on crates.io without the compiler,
so the next `sabiruby` needs a new version number.
