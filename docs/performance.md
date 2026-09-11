# Performance notes

Measured numbers live in [`bench.md`](bench.md) (`tools/bench.sh`). This file records the
reasoning and predictions behind performance decisions, so they can be checked against
measurements later. Dates are when the prediction was written; each prediction should get a
"measured" line when the experiment is done.

## Value representation and where the "window" sits (2026-09-11)

Context: `Value` is a 16-byte Rust enum (`Nil | False | True | Int(i64) | Float(f64) | Sym | Obj(ObjId)`).
mruby's Word Boxing packs a value into 8 bytes. The compatibility question is settled: the test suite
passes without boxing except for the identity of NaN (see `../README.md`). The remaining question is speed and
memory. Two ways to prepare for an 8-byte representation were considered:

* **A** — every use goes through accessors (`v.as_int()`, `match v.kind()`), so the representation is hidden everywhere.
* **B** — only *storage* is opaque: registers, array elements, hash entries, instance variables, environments, constants
  and globals hold `Slot`; code that computes still works on `Value`. Conversions happen at `slot.get()` / `Slot::from(v)`.
  **B was chosen** (one rule, storage vs computation; the storage slots are also exactly what a garbage collector scans).

### Predictions

1. **Introducing the window with the representation unchanged costs nothing.** `Slot` is `#[repr(transparent)]` over `Value`
   and `get`/`from` are trivially inlined, so the generated code should be identical. Check: `bench.md` before/after the
   refactor should agree within noise.
2. **After switching `Slot` to 8 bytes, the gain comes from memory bandwidth** (registers, arrays and hashes halve in size),
   and A and B would gain the same amount from that. The A-vs-B difference is an unpack at every use (A) versus one unpack at
   the storage boundary (B); the unpacked `Value` lives in machine registers or the native stack, so this should be within noise.
   B's only extra cost is converting arguments passed to native methods; A's only extra cost is re-tagging when values are
   passed around. Neither should be visible in `bench.md`.
3. **Boxing is second-order compared with the obvious waste.** `bm_so_lists` (15.8x slower than the reference) is dominated by
   natives that copy whole arrays (`items()`) and by `Array#shift` being O(n) (mruby shares the buffer and shifts a pointer).
   Expectation: fixing those brings `so_lists` to a few times the reference; boxing on top of that is worth 1.1x–1.3x on
   array-heavy code and little on `bm_fib`-style integer code.
4. **What would change the decision**: if `bench.md` after (3) still shows memory-bound behaviour (large arrays, WASM/MCU
   memory limits), try an 8-byte `Slot` (tagged `u64`: 3 tag bits, 61-bit integers, ObjId index, floats either NaN-boxed or
   heap-allocated) and measure. The refactor B makes that experiment a one-day change confined to `value.rs` and the
   `Slot` conversions.

### Measured

* 2026-09-11, prediction 1 — `Slot` introduced at every storage site (registers, arrays, hashes, ivars, envs, constants,
  globals, ranges), representation unchanged. `bench.md` before → after: `bm_fib` 3.45x → 3.40x, `bm_so_lists` 15.75x → 15.61x,
  `bm_so_mandelbrot` 1.80x → 1.82x (best of 3 each). Within noise, as predicted. Test suite unchanged (805/833, 15 fixtures).

## Garbage collector (2026-09-11)

Design in [`gc.md`](gc.md). Prediction (from `gc-plan.md`): the per-instruction cost is one `bool` test at the loop head,
so the three benchmarks stay within noise (a slowdown over 5% means something heavy went into `alloc`); memory of an
allocating loop becomes bounded.

### Measured

* Benchmarks, SabiRuby before (`d81acb2`) → after, best of 5 runs interleaved on the same host (ms):
  `bm_fib` 6162 → 6233 (+1.1%), `bm_so_lists` 3778 → 3763 (−0.4%), `bm_so_mandelbrot` 1639 → 1694 (+3.3%).
  `bm_fib` and `bm_so_mandelbrot` never collect (`so_lists`: 5 collections, 0.7 ms), so the difference is the loop-head
  test: a build without it ran mandelbrot in 1656 ms in the same session (about 2.7% of the 3.3%). Within the 5% budget;
  accepted in review (2026-09-11).
* Candidate, not done: fold the loop-head GC test into the step-budget test (one branch for both). If tried, it is
  separate work with its own before/after measurement: the merged branch ties the step semantics (pausing when the budget
  runs out, which must happen only in the outermost loop) to the collection test, and that touches the paths where a
  fiber stops. Not worth losing today's separation for ~2.7% on the tightest arithmetic loop.
* `bench/src/gc_churn.rb` (1 000 000 iterations of `a = [i, "x" * 10, {k: i}]`, `sabiruby run --stats`):

  | | time | max RSS | live at the end |
  |---|---:|---:|---:|
  | before (no collector) | 631 ms | 1 346 712 KB | 4 000 347 |
  | after | 349 ms | 4 452 KB | 2 994 |

  976 collections, 52 ms in total, so about 53 µs per collection (a few thousand live objects plus the ~350 of mrblib).
  `live` saw-tooths between about 350 right after a collection and 350 + 4096 (the minimum interval) before the next;
  the heap table stays at 4 438 slots. The run is faster than before because the heap no longer grows (no reallocation
  of a 4-million-entry `Vec`, better cache behaviour).
* Stress mode (`SABIRUBY_GC_STRESS=1`, a collection at every boundary after an allocation): the whole test suite takes
  1.5 s instead of 0.13 s (release), 30 s in a debug build; the results are identical.

## Known structural costs (2026-09-11)

* `Value` 16 bytes (above).
* Heap = `Vec<HeapObject>` indexed by `ObjId`; allocation pops a free slot or pushes. Mark & sweep, stop the world
  (`gc.md`): a pause is proportional to the heap (about 50 µs for 4 000 slots).
* Native methods copy arrays (`items()`) instead of borrowing; `Array#shift`/`unshift` are O(n).
* Native → VM re-entry (`Vm::funcall`, `call_block`) uses the host stack; `sort` with a block and `Hash#each`-style
  natives pay a frame setup per callback. The Future-native design (see the book's notes) removes this.
* Method lookup walks the class chain with `HashMap` lookups at each node; no inline cache.
* `Hash` is insertion-ordered linear search over cached hash codes (mruby's AR mode); no hash table for large hashes yet.
