# SabiRuby

A Rust implementation of the [mruby](https://github.com/mruby/mruby) virtual machine.
It executes RITE bytecode (`.mrb` files produced by mruby 4.1's `mrbc`) and aims at
behavioural compatibility with mruby 4.1.0, verified against the reference
implementation rather than against a spec.

SabiRuby is the VM only. Compilation still uses the reference `mrbc`. The Bevy
integration lives in a separate crate, [`rubevy`](../rubevy).

The design follows the book *Deep dive into mruby* (in Japanese): the register
layout (`R0` of the callee is `R[a]` of the caller), `OP_ENTER`, environments,
the `RBreak`-based unwinding through `ensure`, `OP_CALL` as the body of
`Proc#call`, and so on are ported from the book's description of `src/vm.c`.

## Status (2026-09-11, v0)

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
  [`docs/fibers.md`](docs/fibers.md)) and mruby-enumerator (its `mrblib` embedded as
  `src/mrblib_enumerator.mrb`). `send`/`__send__` from bytecode dispatch in place, as in
  mruby, so a `Fiber.yield` behind them is not a native boundary.

Not yet: garbage collection (the heap only grows), bigint, `$~`/`$_`, the other mrbgems
(`sprintf`, `*-ext`, …), encodings. Native code may re-enter the VM (`Vm::funcall`,
`Vm::call_block`); the Future-native design is a later step.

Known deviations from the reference: a NaN has no identity (Floats are immediates, so two
NaNs made apart are `equal?`), and a hash pattern whose keys mutate the subject during
matching is not detected.

## Rules

* **`no_std` + `alloc`.** The library must not use `std::` (only `core::`/`alloc::`,
  `hashbrown` for hash maps, `libm` for float math). `tools/check_no_std.sh` builds the
  library for `thumbv7em-none-eabi` and greps for `std::`; CI runs it. The `std` feature
  (default) only enables the CLI and the tests.
* **Behaviour is checked against the reference, not against memory.** Every claim about
  mruby semantics is verified with the 4.1.0-rc binary (fixtures, test suite below).

## Verification

`tests/fixtures/*.rb` are compiled and run by the reference mruby 4.1.0-rc
(Docker image `kishima/mruby:4.1.0-rc`, see `tools/fixtures.sh`); `.out` holds the
reference stdout, `.dump` the `mrbc --verbose` listing. `cargo test` runs every `.mrb`
on SabiRuby and compares stdout byte for byte. 15 fixtures pass; `enumerator` is `#[ignore]`d
until mruby-enum-ext (`each_slice`, `each_cons`) is ported.

mruby's own test suite (`test/t`, 833 assertions on 4.1.0-rc) plus the tests of the ported
gems (`gem_*`, 77 assertions: mruby-fiber's `fiber.rb`/`fiber2.rb` and mruby-enumerator's)
passes 882 of 910 (see [`docs/mrbtest.md`](docs/mrbtest.md)); every gem assertion passes.
The rest: 16 need the C test fixtures of mruby-test (`env.c`, `vformat.c`, `sysfail.c`,
`ary_shared.c`) or a real garbage collector (arena tests), 2 are the deviations above, and
the remaining ones are skips the reference makes too (bigint, regexp, build-dependent).
`tools/mrbtest.sh` compiles the gem tests and gem mrblibs too (`GEMS` in the script).

The reference image includes some mrbgems (array-ext, hash-ext, compar-ext, …); the
fixtures stay on core behaviour, and the few gem methods that were convenient (`Array#to_h`,
`zip`, `fetch`, `Hash#fetch`, `Comparable#clamp`) are implemented natively and noted as such.

### mruby's own test suite

`tools/mrbtest.sh` copies `test/assert.rb` and `test/t/*.rb` from the reference tree,
compiles them with the reference `mrbc` (Docker) and runs each file on a fresh VM
(`sabiruby mrbtest`). The result is written to [`docs/mrbtest.md`](docs/mrbtest.md):
a per-file table of `report` counts (ok / ko / crash / warn / skip) and the list of
opcodes the suite never executed. `tests/mrbtest/baseline.txt` records the `ok` count
per file and `cargo test` fails if any file drops below it; refresh it with
`tools/mrbtest.sh --update` after an improvement. `tools/mrbtest.sh -v hash` prints the
individual assertion messages of one file.

Errors the VM raises for missing features surface as `NotImplementedError`, so the
suite keeps going and the table shows them as "crash".

## Performance

`tools/bench.sh` runs mruby's own `benchmark/*.rb` on the reference `mruby` and on SabiRuby
and writes [`docs/bench.md`](docs/bench.md) (best of 3, plus instruction counts and
ns/instruction from `sabiruby run --stats`). The ratio column is the number to watch; the
first baseline (2026-09-11) is 1.8x–3.5x slower on arithmetic and 16x on array-heavy code.
The value representation (16-byte enum) and the heap (index into a `Vec`, no GC) are the
known structural costs; measure before changing them. Storage (registers, array elements,
hash entries, ivars, envs, constants, globals) holds `Slot`; computation works on `Value`;
`slot.get()` / `Slot::from(v)` are the only crossings, so an 8-byte representation can be
tried by changing `value.rs` alone. Predictions and measurements: [`docs/performance.md`](docs/performance.md).
Exception/break unwinding without longjmp: [`docs/exceptions.md`](docs/exceptions.md).

## Usage

```
cargo run -- run  tests/fixtures/klass.mrb   # execute
cargo run -- dump tests/fixtures/klass.mrb   # instruction listing
cargo run -- mrbtest tests/mrbtest/assert.mrb tests/mrbtest/hash.mrb   # test suite
```

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
| `src/object.rs` | heap objects (classes, procs, envs, strings, arrays, hashes) |
| `src/builtins/` | native methods per class |
| `src/mrblib.mrb` | mruby's `mrblib/*.rb`, compiled by the reference `mrbc` |
| `tests/fixtures/` | reference programs, bytecode and expected output |
| `tools/fixtures.sh` | regenerates the fixtures and `mrblib.mrb` with Docker |
| `src/mrbtest.rs`, `tests/mrbtest/` | runner and compiled files of mruby's test suite |
| `tools/mrbtest.sh`, `tools/check_no_std.sh` | test-suite report, no_std rule |

## License

MIT. `src/mrblib.mrb` is compiled from mruby's `mrblib`, which is MIT licensed
(Copyright (c) 2010- mruby developers).
