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

Not yet: keyword parameters (`OP_ENTER` kdict, `KEY_P`/`KEYEND`/`KARG`; keyword *arguments*
at call sites are packed into a trailing Hash like mruby does for callees without keyword
parameters), Fiber, garbage collection (the heap only grows), bigint, `$~`/`$_`,
`Comparable`/`Enumerable` gems, `sprintf`, encodings. Native code may re-enter the VM
(`Vm::funcall`, `Vm::call_block`); the Future-native design is a later step.

## Verification

`tests/fixtures/*.rb` are compiled and run by the reference mruby 4.1.0-rc
(Docker image `kishima/mruby:4.1.0-rc`, see `tools/fixtures.sh`); `.out` holds the
reference stdout, `.dump` the `mrbc --verbose` listing. `cargo test` runs every `.mrb`
on SabiRuby and compares stdout byte for byte. 13 fixtures pass; `kwargs` is `#[ignore]`d
until keyword parameters are implemented.

The reference image includes some mrbgems (array-ext, hash-ext, compar-ext, …); the
fixtures stay on core behaviour, and the few gem methods that were convenient (`Array#to_h`,
`zip`, `fetch`, `Hash#fetch`, `Comparable#clamp`) are implemented natively and noted as such.

## Usage

```
cargo run -- run  tests/fixtures/klass.mrb   # execute
cargo run -- dump tests/fixtures/klass.mrb   # instruction listing
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

## License

MIT. `src/mrblib.mrb` is compiled from mruby's `mrblib`, which is MIT licensed
(Copyright (c) 2010- mruby developers).
