# Code size, and what a Cargo feature takes off it

What the VM costs in bytes, and what leaving a gem out gives back. The first (and so far only)
gem behind a feature is mruby-regexp (`Cargo.toml`, feature `regexp`, on by default).

Measured 2026-09-17 at `96221fb`, on Linux x86_64, toolchain `stable` (rustc 1.97.0), with
**`arm-none-eabi-size` 2.38** (GNU binutils, Berkeley format: the `text` column is `.text` plus
the read-only sections, which is where the embedded `.mrb` bytecode lands) and the host
**`size`** of the same binutils for x86_64.

**Every number here is a `codegen-units = 1` build**, which `96221fb` made the release default
(`../design/optimizations.md` 4). It is not a detail of the measurement: one unit is 8-12%
smaller than the sixteen the earlier numbers here were taken with, and the whole comparison is
in [What one code generation unit takes off](#what-one-code-generation-unit-takes-off) below.
The table before this re-measurement was taken at `eec1d23` with 16 units; the tree has also
moved a little since, so the two effects are separated there by measuring both at `96221fb`.

## thumbv7em-none-eabi, release

The library is built with `cargo build --lib --release --target thumbv7em-none-eabi
--no-default-features --features "…"`, once with `CARGO_PROFILE_RELEASE_OPT_LEVEL=z` and once
with the crate's own release profile (`opt-level = 3`, `codegen-units = 1`).

There is no link step for a library, so these are the sizes of the **archives**: `own` is
`libsabiruby-*.rlib`, `graph` is every `.rlib` cargo produced for the target — the VM plus
`hashbrown`, `foldhash`, `libm`, and, with the feature on, `regex-automata` and `regex-syntax`.
A linked firmware is smaller than `graph` (the linker drops what nothing reaches) and never
larger, so `graph` is the upper bound and the difference between two rows is the honest number.

| build | `opt-level` | own `text` | graph `text` | crates in graph |
|---|---|---:|---:|---|
| `utf8`, `regexp` | `z` | 652,789 | 1,254,310 | 6 |
| `utf8` (no regexp) | `z` | 601,938 | **680,867** | 4 |
| `utf8`, `regexp` | 3 | 1,030,524 | 1,736,317 | 6 |
| `utf8` (no regexp) | 3 | 946,566 | **1,044,673** | 4 |

Leaving mruby-regexp out takes **573,443 bytes (−45.7%)** off the `opt-level = "z"` graph and
691,644 (−39.8%) off the `opt-level = 3` one. Only 50,851 of those bytes (8.9%) are SabiRuby's
own code; the other 522,592 are `regex-automata` and `regex-syntax`, which nothing else in the
VM uses. Put the other way: **the VM without the pattern engine is little more than half the
size of the VM with it**, and the gem's Rust is a small part of that. The proportions are the
ones the 16-unit build showed (−46.6% and −41.6%); one unit moved the bytes, not the story.

## x86_64-unknown-linux-gnu, release

Second data point, and the only one with a **linker** in it: the `sabiruby` command is a real
binary, so its numbers are after dead-code elimination.

| artifact | with regexp | without regexp | difference |
|---|---:|---:|---:|
| `libsabiruby.rlib`, own `text` | 1,563,310 | 1,421,158 | −142,152 (−9.1%) |
| `libsabiruby.rlib`, graph `text` | 2,388,778 | 1,512,981 | −875,797 (−36.7%) |
| `target/release/sabiruby`, file | 4,678,600 | 3,475,304 | **−1,203,296 (−25.7%)** |
| `target/release/sabiruby`, `.text` | 3,858,226 | 2,902,830 | −955,396 (−24.8%) |

The command links the reference compiler (prism and mruby's codegen, as C) as well as the VM,
so a quarter of a binary that carries a whole Ruby compiler is what the pattern engine costs.
It was a third before the profile changed: one code generation unit takes more off the build
*with* the engine than off the one without it, so the gem's share of the binary shrank too.

## What one code generation unit takes off

`96221fb` set `[profile.release] codegen-units = 1`. Both columns below were measured at that
commit on the same afternoon, the 16-unit one by overriding the profile
(`CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16`), so the difference is the profile and nothing else.

| artifact | 16 units | 1 unit | difference |
|---|---:|---:|---:|
| thumbv7em graph, `utf8 regexp`, `z` | 1,397,505 | 1,254,310 | −143,195 (−10.2%) |
| thumbv7em graph, `utf8`, `z` | 747,280 | 680,867 | −66,413 (−8.9%) |
| thumbv7em graph, `utf8 regexp`, 3 | 1,968,062 | 1,736,317 | −231,745 (−11.8%) |
| thumbv7em graph, `utf8`, 3 | 1,148,604 | 1,044,673 | −103,931 (−9.0%) |
| x86_64 `libsabiruby.rlib`, own `text` | 1,690,417 | 1,563,310 | −127,107 (−7.5%) |
| x86_64 `sabiruby`, `.text` | 4,174,358 | 3,858,226 | −316,132 (−7.6%) |
| x86_64 `sabiruby`, file | 5,621,344 | 4,678,600 | **−942,744 (−16.8%)** |

Code that lives in one unit is emitted once. With sixteen, a generic instantiation or an
inlinable function that several units reach is emitted in each of them and the linker keeps one
copy per unit in the archive; the `text` columns above are the archive's, which is why the
libraries lose 7-12%. The linked command loses 7.6% of `.text` but 16.8% of the **file**, the
difference being the symbol and unwind tables that the duplicate copies also carried.

The 16-unit numbers here are within 1% of the ones this file recorded at `eec1d23`
(e.g. 1,391,312 against 1,397,505 for the first row), which is the tree moving between the two
commits; the 8-12% is the profile.

## Where the bytes are, per gem

From `arm-none-eabi-nm --print-size --demangle` over the `opt-level = "z"`, `utf8 regexp` rlib,
summed by the module a symbol's demangled name starts with (generic instantiations land under
the module that named the type, so this is an attribution, not a partition).

**This is the one table above that was not re-measured at `96221fb`**: it is still the 16-unit
`eec1d23` rlib. Re-running the attribution did not reproduce these figures closely enough to
mix the two (binutils 2.38 does not demangle Rust's v0 names, so how the leftover `_R…` symbols
are attributed changes every row), and a half-matching column is worse than a dated one. What
the table is used for is the ordering — which module is large, and which gem drags a crate in
behind it — and that does not turn on the profile.

| module | `text` |
|---|---:|
| `builtins::ext_regexp` (the Ruby surface of mruby-regexp) | 35,144 |
| `Vm::exec_frames` (the instruction loop) | 17,420 |
| `builtins::numeric` | 11,632 |
| `regexp` (the translation to `regex-automata`) | 9,442 |
| `builtins::ext_pack` | 9,020 |
| `builtins::ext_task` | 8,460 |
| `builtins::ext_sprintf` | 8,388 |
| `builtins::ext_random` | 5,168 |
| `builtins::ext_time` | 2,900 |
| `builtins::ext_strftime` | 1,308 |

This is the table that decided **not** to give `random`, `time` and `pack` a feature of their
own for now (`docs/plans/from-mrubyedge-plan.md`, item 4): together they are about 17 KB, some
2% of the smallest build, against mruby-regexp's 47%. A gem is worth a feature when it drags a
crate in behind it, and mruby-regexp is the only one that does.

## Reproducing

```sh
# thumbv7em, the two feature sets, either optimisation level.
# Prefix any of these with CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 for the 16-unit column.
rm -rf target/thumbv7em-none-eabi
CARGO_PROFILE_RELEASE_OPT_LEVEL=z cargo build --lib --release \
    --target thumbv7em-none-eabi --no-default-features --features "utf8 regexp"
arm-none-eabi-size -t target/thumbv7em-none-eabi/release/deps/*.rlib | tail -1

# x86_64: the command, which is linked
cargo build --release -p sabiruby-cli                                  # with regexp
cargo build --release -p sabiruby-cli --no-default-features --features utf8
size target/release/sabiruby
```

`arm-none-eabi-size` comes with a GNU arm toolchain; `llvm-size` from `rustup component add
llvm-tools` reads the same archives and prints the same `text` column.
