# SabiRuby

A Rust implementation of the [mruby](https://github.com/mruby/mruby) virtual machine.
It executes RITE bytecode (`.mrb` files produced by `mrbc`) and aims at behavioural
compatibility with mruby 4.1. mruby 4.1.0 itself is not released yet: the reference everything
here is checked against is the release candidate **4.1.0-rc** (tag `4.1.0-rc`, commit
`3cf73ee`), and verification is against that binary rather than against a spec.

The VM runs bytecode only and is pure Rust (`no_std`). Three crates live in this repository:

| crate | what | |
|---|---|---|
| [`sabiruby`](https://crates.io/crates/sabiruby) | the VM library | pure Rust, `no_std` + `alloc`, wasm |
| [`sabiruby-compiler`](https://github.com/kishima/sabiruby/tree/main/compiler) | the reference compiler (mruby 4.1.0-rc's `mruby-compiler`: Prism as its parser, mruby's code generator) built as C; output byte-identical to `mrbc` | needs a C compiler |
| [`sabiruby-cli`](https://github.com/kishima/sabiruby/tree/main/cli) | the `sabiruby` command: `sabiruby run foo.rb`, `-e`, `compile`, `dump` | depends on both |

The Bevy integration lives in a separate crate, [`rubevy`](https://github.com/kishima/rubevy).
Try it in the browser: **[SabiRuby Playground](https://kishima.github.io/sabiruby-playground/)**
(the VM and the reference compiler as WebAssembly; [`docs/playground.md`](https://github.com/kishima/sabiruby/blob/main/docs/playground.md)).

The design follows the book *Deep dive into mruby* (in Japanese): the register
layout (`R0` of the callee is `R[a]` of the caller), `OP_ENTER`, environments,
the `RBreak`-based unwinding through `ensure`, `OP_CALL` as the body of
`Proc#call`, and so on are ported from the book's description of `src/vm.c`.

## Status (2026-09-12, 0.2.0)

* RITE 04.00 reader (IREP / LVAR; DBG skipped), all 119 opcodes decoded, `EXT1..3` handled.
* Interpreter with methods, blocks/closures (attached/detached environments), `super`
  (incl. `ARGARY`), `rescue`/`ensure` with correct non-local exits (`return`/`break`/`JMPUW`
  through `ensure`), class/module/singleton classes, class variables, `method_missing`.
* Native core classes: Object, Module, Class, Kernel, NilClass/TrueClass/FalseClass,
  Integer, Float, Symbol, String (bytes, no encoding), Array, Hash, Range, Proc, Exception hierarchy.
* mruby's own `mrblib/*.rb` (Enumerable, Comparable, `Array#each`, `Integer#times`, …) is
  compiled by the reference `mrbc` and embedded (`src/mrblib.mrb`), so those run as bytecode.
* Step execution with an instruction budget (`Vm::start` / `Vm::step`) for host loops.

* Keyword parameters, visibility (`private`/`protected`/`module_function`), `prepend`,
  hooks (`inherited`, `included`, `method_added`, …), `defined?`, frozen objects.
* Gems: mruby-fiber (`Fiber`, contexts switched like mruby's `mrb->c`, see
  [`docs/fibers.md`](https://github.com/kishima/sabiruby/blob/main/docs/fibers.md)), mruby-enumerator, and mruby-array-ext, -enum-ext,
  -hash-ext, -range-ext, -string-ext, mruby-sprintf, -metaprog, -proc-ext, -method (natives
  in `src/builtins/ext_*.rs`, the gems' Ruby parts embedded as `src/mrblib_<gem>.mrb` and
  loaded in the reference gembox order; see [`docs/gems.md`](https://github.com/kishima/sabiruby/blob/main/docs/gems.md)). `send`/`__send__`
  from bytecode dispatch in place, as in mruby, so a `Fiber.yield` behind them is not a native
  boundary.
* Garbage collection: stop-the-world mark & sweep with a free list, run at instruction
  boundaries and never while a native is on the host stack (natives need no arena; a host
  keeping objects across calls uses `Vm::gc_register`). `GC.start`/`enable`/`disable`,
  `interval_ratio`, `malloc_threshold`, `GC.stat[:live]` are real. `SABIRUBY_GC_STRESS=1`
  collects after every allocation. See [`docs/gc.md`](https://github.com/kishima/sabiruby/blob/main/docs/gc.md).
* Compiler: the reference `mruby-compiler` (Prism as its parser, mruby's own code generator)
  linked as C, in the `sabiruby-compiler`
  crate; the VM crate does not depend on it, the `sabiruby` command (`sabiruby-cli`) does.
  Output is byte-identical to `mrbc` for every `.rb` in the repository. See
  [`docs/compiler.md`](https://github.com/kishima/sabiruby/blob/main/docs/compiler.md).

Not yet: bigint, the special variables `$~`/`$_` and `$!` (nil even inside `rescue`; use
`rescue => e`), `eval`/`require` (design notes below), the remaining mrbgems (`io`, `time`,
`math`, `struct`, `compar-ext`, …), encodings. Native code may re-enter the VM
(`Vm::funcall`, `Vm::call_block`); the Future-native design is a later step.

Known deviations from the reference: a NaN has no identity (Floats are immediates, so two
NaNs made apart are `equal?`), and a hash pattern whose keys mutate the subject during
matching is not detected.

## Rules

* **`no_std` + `alloc`.** The library must not use `std::` (only `core::`/`alloc::`,
  `hashbrown` for hash maps, `libm` for float math). `tools/check_no_std.sh` builds the
  library for `thumbv7em-none-eabi` and greps for `std::`; CI runs it. The `std` feature
  (default) only adds `std::error::Error` for `VmError`. The C compiler and the command line
  tool are the separate crates `sabiruby-compiler` and `sabiruby-cli`.
* **Behaviour is checked against the reference, not against memory.** Every claim about
  mruby semantics is verified with the 4.1.0-rc binary (fixtures, test suite below).

## Verification

`tests/fixtures/*.rb` are compiled and run by the reference mruby 4.1.0-rc
(Docker image `kishima/mruby:4.1.0-rc`, see `tools/fixtures.sh`); `.out` holds the
reference stdout, `.dump` the `mrbc --verbose` listing. `cargo test` runs every `.mrb`
on SabiRuby and compares stdout byte for byte. All 17 fixtures pass (`gc.rb` also under `SABIRUBY_GC_STRESS=1`).

mruby's own test suite (`test/t`, 833 assertions on 4.1.0-rc) plus the tests of the ported
gems (`gem_*`, 394 assertions) passes 1185 of 1227 (see [`docs/mrbtest.md`](https://github.com/kishima/sabiruby/blob/main/docs/mrbtest.md),
reasons for the rest in [`docs/mrbtest-notes.md`](https://github.com/kishima/sabiruby/blob/main/docs/mrbtest-notes.md)); of the gem
assertions only the two NaN identity tests fail, the C-fixture ones crash and the UTF-8/DBG
ones skip.
The rest: 11 need the C test fixtures of mruby-test (`env.c`, `vformat.c`, `sysfail.c`,
`ary_shared.c`), 2 are the deviations above, 1 is `(1..).last`, where the core test and
mruby-range-ext disagree (the reference `mruby` crashes on it too), and the remaining ones are
skips the reference makes too (bigint, regexp, build-dependent).
`tools/mrbtest.sh` compiles the gem tests and gem mrblibs too (`GEMS` in the script).

The reference image includes the default gembox (array-ext, hash-ext, compar-ext, …). The
gems listed under Status are ported; of the others, only `Comparable#clamp` (mruby-compar-ext)
is provided, natively.

### SabiRuby's own tests

`tests/custom/<case>.rb` are tests written for SabiRuby itself: behaviour the
reference `mruby` gets wrong in 4.1.0-rc (with the upstream fix named), things
the reference test suite does not cover, and features planned but not built
yet. Each case has a hand-decided `.expected` (its header says from what:
CRuby, an mruby master commit, or the reference), the reference output
`.rc.out` to show where they differ on purpose, and a `.mrb` compiled by the
reference `mrbc` (`tools/custom.sh`, Docker). A header line
`# pending: <feature>` marks a case that must fail until that feature exists;
the runner (`tests/custom.rs`, part of `cargo test`) fails when a pending case
starts passing, so the marker is removed with the feature. The fixtures and
mruby's suite above are the baseline and are not changed by this.

### mruby's own test suite

`tools/mrbtest.sh` copies `test/assert.rb` and `test/t/*.rb` from the reference tree,
compiles them with the reference `mrbc` (Docker) and runs each file on a fresh VM
(`sabiruby mrbtest`). The result is written to [`docs/mrbtest.md`](https://github.com/kishima/sabiruby/blob/main/docs/mrbtest.md):
a per-file table of `report` counts (ok / ko / crash / warn / skip) and the list of
opcodes the suite never executed. `tests/mrbtest/baseline.txt` records the `ok` count
per file and `cargo test` fails if any file drops below it; refresh it with
`tools/mrbtest.sh --update` after an improvement. `tools/mrbtest.sh -v hash` prints the
individual assertion messages of one file.

Errors the VM raises for missing features surface as `NotImplementedError`, so the
suite keeps going and the table shows them as "crash".

## Performance

`tools/bench.sh` runs mruby's own `benchmark/*.rb` on the reference `mruby` and on SabiRuby
and writes [`docs/bench.md`](https://github.com/kishima/sabiruby/blob/main/docs/bench.md) (best of 3, plus instruction counts and
ns/instruction from `sabiruby run --stats`). The ratio column is the number to watch; the
first baseline (2026-09-11) is 1.8x–3.5x slower on arithmetic and 16x on array-heavy code.
The value representation (16-byte enum) and the heap (index into a `Vec`) are the
known structural costs; measure before changing them. Storage (registers, array elements,
hash entries, ivars, envs, constants, globals) holds `Slot`; computation works on `Value`;
`slot.get()` / `Slot::from(v)` are the only crossings, so an 8-byte representation can be
tried by changing `value.rs` alone. Predictions and measurements: [`docs/performance.md`](https://github.com/kishima/sabiruby/blob/main/docs/performance.md).
Exception/break unwinding without longjmp: [`docs/exceptions.md`](https://github.com/kishima/sabiruby/blob/main/docs/exceptions.md).
Compiler: [`docs/compiler.md`](https://github.com/kishima/sabiruby/blob/main/docs/compiler.md) (the plan: [`docs/compiler-plan.md`](https://github.com/kishima/sabiruby/blob/main/docs/compiler-plan.md)).
GC: [`docs/gc.md`](https://github.com/kishima/sabiruby/blob/main/docs/gc.md) (the plan it was built from: [`docs/gc-plan.md`](https://github.com/kishima/sabiruby/blob/main/docs/gc-plan.md)).
eval / require (not implemented; design notes, with PicoRuby's approach as the reference): [`docs/eval-require-plan.md`](https://github.com/kishima/sabiruby/blob/main/docs/eval-require-plan.md).
UTF-8 strings (not implemented; plan): [`docs/utf8-plan.md`](https://github.com/kishima/sabiruby/blob/main/docs/utf8-plan.md). Remaining gems and their order: [`docs/gems.md`](https://github.com/kishima/sabiruby/blob/main/docs/gems.md).

## Usage

The library: `cargo add sabiruby` (`default-features = false` for `no_std`). The command:
`cargo install sabiruby-cli` (installs `sabiruby`; building it compiles the C sources of the
reference compiler, about 2 s in a debug build and 7 s in a release build, on one core).
In this repository:

The switches are the reference `mruby` command's (`sabiruby -h` lists them); `compile` is `mrbc`:

```
sabiruby foo.rb arg1 arg2      # Ruby source or a .mrb; the arguments go to ARGV
sabiruby -e 'p [1, 2].sum'     # one line of script (-e may be repeated)
echo 'puts 1' | sabiruby       # no program file: read it from standard input
sabiruby -c foo.rb             # check syntax only ("Syntax OK")
sabiruby -v foo.rb             # version, then the instruction listing, then run
sabiruby -b foo.mrb            # bytecode only; -d sets $DEBUG; --stats prints instructions/time/GC
sabiruby compile foo.rb -o foo.mrb   # like mrbc (-g, -c, --remove-lv, --no-ext-ops, --no-optimize)
sabiruby dump foo.rb           # instruction listing (.rb or .mrb)
sabiruby mrbtest tests/mrbtest/assert.mrb tests/mrbtest/hash.mrb   # test suite
```

A subcommand name wins over a file of the same name: run `./compile` (or `sabiruby run compile`)
for a program file called `compile`. In this repository, use `cargo run -p sabiruby-cli -- …`.

```rust
let mut vm = sabiruby::Vm::with_mrblib()?;   // core library loaded
vm.load_and_run(&bytes)?;                   // run a RITE binary to completion
let out = vm.take_output();                 // what puts/p printed

// or stepped, e.g. once per frame:
let irep = vm.load(&bytes)?;
vm.start(irep);
loop {
    match vm.step(10_000)? {               // at most 10k instructions
        sabiruby::Step::Paused => { /* next frame */ }
        sabiruby::Step::Finished(v) => break,
    }
}
```

## Layout

| path | content |
|---|---|
| `src/rite.rs` | RITE binary reader |
| `src/opcode.rs` | opcode table generated from mruby's `ops.h` |
| `src/vm.rs` | interpreter loop, frames, environments, unwinding |
| `src/object.rs` | heap objects (classes, procs, envs, strings, arrays, hashes), mark & sweep |
| `src/builtins/` | native methods per class |
| `src/mrblib.mrb` | mruby's `mrblib/*.rb`, compiled by the reference `mrbc` |
| `tests/fixtures/` | reference programs, bytecode and expected output |
| `tools/fixtures.sh` | regenerates the fixtures and `mrblib.mrb` with Docker |
| `src/mrbtest.rs`, `tests/mrbtest/` | runner and compiled files of mruby's test suite |
| `compiler/` | crate `sabiruby-compiler`: the vendored reference compiler, C shim, golden tests |
| `cli/` | crate `sabiruby-cli`: the `sabiruby` command |
| `tools/vendor_compiler.sh` | refreshes `compiler/vendor/` from the reference tree |
| `tools/mrbtest.sh`, `tools/check_no_std.sh` | test-suite report, no_std rule |

## License

MIT (`LICENSE`). `src/mrblib.mrb` and `src/mrblib_*.mrb` are compiled from mruby's `mrblib`
and the Ruby parts of its bundled gems, and `tests/mrbtest/src/` holds copies of mruby's test
suite; those are MIT licensed, Copyright (c) 2010- mruby developers (`LICENSE-mruby`).
