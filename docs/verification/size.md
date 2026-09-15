# Code size, and what a Cargo feature takes off it

What the VM costs in bytes, and what leaving a gem out gives back. The first (and so far only)
gem behind a feature is mruby-regexp (`Cargo.toml`, feature `regexp`, on by default).

Measured 2026-09-16 at `eec1d23` + the `regexp` feature, on Linux x86_64, toolchain `stable`
(rustc 1.97.0), with **`arm-none-eabi-size` 2.38** (GNU binutils, Berkeley format: the `text`
column is `.text` plus the read-only sections, which is where the embedded `.mrb` bytecode
lands) and the host **`size`** of the same binutils for x86_64.

## thumbv7em-none-eabi, release

The library is built with `cargo build --lib --release --target thumbv7em-none-eabi
--no-default-features --features "…"`, once with `CARGO_PROFILE_RELEASE_OPT_LEVEL=z` and once
with the crate's own release profile (the crate sets no `[profile.release]`, so `opt-level = 3`).

There is no link step for a library, so these are the sizes of the **archives**: `own` is
`libsabiruby-*.rlib`, `graph` is every `.rlib` cargo produced for the target — the VM plus
`hashbrown`, `foldhash`, `libm`, and, with the feature on, `regex-automata` and `regex-syntax`.
A linked firmware is smaller than `graph` (the linker drops what nothing reaches) and never
larger, so `graph` is the upper bound and the difference between two rows is the honest number.

| build | `opt-level` | own `text` | graph `text` | crates in graph |
|---|---|---:|---:|---|
| `utf8`, `regexp` | `z` | 713,398 | 1,391,312 | 6 |
| `utf8` (no regexp) | `z` | 659,400 | **742,467** | 4 |
| `utf8`, `regexp` | 3 | 1,132,499 | 1,952,800 | 6 |
| `utf8` (no regexp) | 3 | 1,033,461 | **1,140,446** | 4 |

Leaving mruby-regexp out takes **648,845 bytes (−46.6%)** off the `opt-level = "z"` graph and
812,354 (−41.6%) off the `opt-level = 3` one. Only 53,998 of those bytes (8.3%) are SabiRuby's
own code; the other 594,847 are `regex-automata` and `regex-syntax`, which nothing else in the
VM uses. Put the other way: **the VM without the pattern engine is less than half the size of
the VM with it**, and the gem's Rust is a small part of that.

## x86_64-unknown-linux-gnu, release

Second data point, and the only one with a **linker** in it: the `sabiruby` command is a real
binary, so its numbers are after dead-code elimination.

| artifact | with regexp | without regexp | difference |
|---|---:|---:|---:|
| `libsabiruby.rlib`, own `text` | 1,679,587 | 1,519,862 | −159,725 (−9.5%) |
| `libsabiruby.rlib`, graph `text` | 2,645,370 | 1,618,002 | −1,027,368 (−38.8%) |
| `target/release/sabiruby`, file | 5,604,312 | 3,862,280 | **−1,742,032 (−31.1%)** |
| `target/release/sabiruby`, `.text` | 4,163,910 | 3,058,686 | −1,105,224 (−26.5%) |

The command links the reference compiler (prism and mruby's codegen, as C) as well as the VM,
so a third of a binary that carries a whole Ruby compiler is what the pattern engine costs.

## Where the bytes are, per gem

From `arm-none-eabi-nm --print-size --demangle` over the `opt-level = "z"`, `utf8 regexp` rlib,
summed by the module a symbol's demangled name starts with (generic instantiations land under
the module that named the type, so this is an attribution, not a partition):

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
# thumbv7em, the two feature sets, either optimisation level
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
