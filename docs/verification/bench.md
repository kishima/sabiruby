# Benchmarks: SabiRuby vs mruby 4.1.0-rc

Measured with `tools/bench.sh` (`bench/src/*.rb`, compiled by the reference `mrbc`; the reference `mruby`
runs inside the Docker image on the same machine, so read the ratio and not the milliseconds). Every
run is pinned to a P core (`--core 2`; this machine mixes P and E cores and an E core is 20% slower).
The raw results of every measurement are in `bench/results/<label>.tsv` with a Markdown view beside
them; `tools/bench_compare.sh a.tsv b.tsv` puts two side by side.

## The baseline and the stages of `docs/plans/host-bridge-plan.md` (2026-09-15)

Best of 5 (the baseline, `e9da768`) and best of 7 (`5c3eb6e`), by category: milliseconds for
SabiRuby, and the ratio to the reference. `ratio (sum)` weighs every benchmark by how long it runs;
`ratio (median)` is the middle of the per-benchmark ratios.

| category | benchmarks | e9da768 (baseline) | ratio (sum) | ratio (median) | 5c3eb6e (stages 0, 1, 3) | ratio (sum) | change |
|---|---|---:|---:|---:|---:|---:|---:|
| whole program | app_json_hash, app_robot, app_tak, bm_fib | 10511 | 3.40x | 2.87x | 11312 | 3.55x | +7.6% |
| data structures | bm_so_lists, ds_array, ds_hash, ds_string | 7259 | 10.85x | 11.43x | 7395 | 10.91x | +1.9% |
| instruction loop | bm_so_mandelbrot, loop_if_branch, loop_times, loop_while_add, vm_optimization_bench | 28402 | 5.71x | 4.09x | 29464 | 5.80x | +3.7% |
| calls | call_args, call_block_yield, call_fiber, call_kwargs | 4392 | 3.40x | 3.31x | 4541 | 3.48x | +3.4% |
| memory | gc_churn, mem_retained, mem_short_lived | 2320 | 1.06x | 2.44x | 2435 | 1.07x | +5.0% |
| **all** | | 52883 | 4.32x | 3.68x | 55147 | 4.41x | +4.3% |

What the baseline says: the slowness is concentrated in **Hash and String** (`ds_hash` 12.9x, `ds_string`
11.4x, `bm_so_lists` 16.4x) while Array is 4.6x, so it is not the cost of moving values in and out of
slots but something in those two kinds. Calls are 3.0–3.7x, instruction loops 3.6–4.2x
(`vm_optimization_bench` alone 7.1x). `mem_retained` — allocating while 20,000 long-lived objects stay
alive — is *faster* than the reference (0.5x): the reference's generational collector re-marks the live
set; at 100,000 objects it took 31 s to SabiRuby's 1.2 s.

### After stages 2, 2b, 4 and 5 (`9fa5b0b`, 2026-09-15)

Baseline `440d4ba` (main after stages 0, 1, 3) against the merged tree, best of 5, reference included
(`bench/results/440d4ba.tsv`, `9fa5b0b.tsv`):

| category | 440d4ba ms | 9fa5b0b ms | change | ratio to mruby (before → after) |
|---|---:|---:|---:|---|
| whole program | 11302 | 8992 | **−20.4%** | 3.53x → **2.88x** |
| data structures | 7413 | 5848 | **−21.1%** | 10.88x → **8.79x** |
| instruction loop | 29218 | 21618 | **−26.0%** | 5.77x → **4.37x** |
| calls | 4557 | 3768 | **−17.3%** | 3.46x → **2.94x** |
| memory | 2417 | 2053 | **−15.1%** | 1.08x → **0.95x** |
| **all** | 54908 | 42279 | **−23.0%** | 4.40x → **3.47x** (median 3.15x) |

Every one of the 22 benchmarks got faster. Stages 4 and 5 add no cost the benchmarks see (they do not
touch the instruction loop; the merged tree measures the same as the perf branch alone, −22.0%).
How each candidate was measured — alternating A/B rather than best of 5, and why — is in
`docs/worklog/2026-09-15-stage2-perf.md`.

### After stage 2c (`9da4724`, 2026-09-15): `Array#shift` in O(1), Hash writes through one door, an index past 16 entries

Against the previous head and against the original baseline, best of 5, reference included
(`bench/results/9da4724.tsv`):

| category | 9fa5b0b → 9da4724 | e9da768 (baseline) → 9da4724 | ratio to mruby now |
|---|---:|---:|---|
| whole program | −3.0% | −17.0% | 2.84x |
| data structures | **−48.9%** | **−58.9%** | 10.85x → **4.57x** |
| instruction loop | −12.8% | −33.7% | 3.83x |
| calls | +0.8% | −13.5% | 2.98x |
| memory | −2.4% | −13.6% | 0.92x |
| **all** | **−14.0%** | **−31.3%** | 4.32x → **3.01x** (median 3.05x) |

`bm_so_lists` 3.94 s → 1.02 s (16.4x → 4.4x), `ds_hash` 12.9x → 7.7x, `ds_string` 11.4x → 4.5x,
`vm_optimization_bench` 7.1x → 4.6x (it holds a 50,000-entry Hash workload). How the two changes
were measured, and the O(n) that came out of hiding when a borrow changed, are in
`docs/worklog/2026-09-15-stage2c-array-hash.md`.

### After stage 2d (`2aa4f13`, 2026-09-15): Hash without the copy per lookup, `vm_optimization_bench` split

`vm_optimization_bench` now sits under "whole program" and its five parts (`vmo_dispatch`, `vmo_arith`,
`vmo_calls`, `vmo_index`, `vmo_objects`) under "instruction loop", so the category sums are not those
of the tables above; `tools/bench_compare.sh` compares the benchmarks the two files share
(`bench/results/2aa4f13.tsv`, best of 5, reference included):

| | 9da4724 → 2aa4f13 (shared benchmarks) | e9da768 (baseline) → 2aa4f13 | ratio to mruby now (sum / median) |
|---|---:|---:|---|
| all | **−13.5%** | **−40.6%** | **2.82x / 3.05x** |

Per benchmark now: `ds_hash` **2.85x** (was 7.65x after 2c, 12.9x at the baseline), `vmo_objects` (a
50,000-entry Hash) 1.92x, `app_robot` 2.08x, `app_json_hash` 2.82x, `bm_fib` 3.05x, `bm_so_lists` 4.35x,
`ds_string` 4.63x. Memory stays 0.94x. What the small-Hash cost is made of, the O(n) that hid in
`hash_sync`, and why `include?`/`count` were fixed although no benchmark calls them, are in
`docs/worklog/2026-09-15-stage2d-perf.md`.

Per stage (each measured against the commit before it):

| stage | commit | what changed | all | notes |
|---|---|---|---:|---|
| 0 | `354b6bb` | opcode decode by table, no `unsafe` | +2.5% | dispatch-bound loops +3–7% (`loop_times` +6.6%), heavy-instruction benchmarks 0% (`bm_so_lists`). The `transmute` had no load; the table has one per instruction. To be won back in stage 2 |
| 1 | `9837294` | the benchmarks themselves | — | no VM change |
| 2 c0 | `5119773` | `Op::from_u8` as a 119-arm `match`, not a table | −1.7% | wins back stage 0; `objdump` shows the match folded to a range check |
| 2 c3 | `68261b0` | `find_method` answers a Copy `MethodRef` on the dispatch path | −2.1% | wins back stage 3: `bm_fib` −5.2%, `call_args` −4.7% |
| 2 c1 | `fcf0772` | the instruction loop reads three fields of `CallInfo`, not a clone of all thirteen | −2.8% | the widest win: nearly everything faster |
| 2 c2 | `815ba7f` | `op_counts` off by default, and a fixed array when on (`Vm::set_op_counting`) | **−5.6%** | 20–25% on dense loops: a `Vec` pointer reload and a store→load dependency per instruction |
| 2 c4 | `4a6826e` | a method cache invalidated by `Heap::method_serial` | −1.3% | every mutable access to a `ClassData` goes through `class_mut`, which bumps the serial |
| 2b | `05f6f28` | `String#[]` without copying the whole string; one borrow per Hash scan | −8.7% | `ds_string` 7.9× faster, `ds_hash` −25 to −32% |
| 4, 5 | `ccc60de`, `c667a9d` | `define_fn`, `ObjKind::Data` | — | not on the instruction loop |
| 2c A | `35d58a5` | `ArrayData { buf, start }`: `shift`/`unshift`/`insert(0)` in O(1) | −6.1% | `bm_so_lists` −73%; `ds_array` first +155% (a hidden O(n) once `&Vec` became `&[Slot]`), fixed by reading the length without copying |
| 2c B1 | `8ba2336` | `HashData` fields private, writes through seven methods | −0.3% | no behaviour change; the door the index needs |
| 2c B2 | `e2b026b` | an index (`hash → entry`, one chain per bucket) past 16 entries | −7.1% | `ds_hash` −19.8%, `vm_optimization_bench` −14.4% |
| 2d | `07659aa` | `hash_sync` asks whether the cached hashes are stale before copying every key | **−11.7%** | `ds_hash` −62.8%, `vmo_objects` −85%, `call_kwargs` −15%: one copy of every key per lookup, gone |
| 2d | `2521b79` | one `key_hash` per store; a String key copied only when inserted | −0.7% | a store to an existing key 138 → 45 ns |
| 2d | `f222934` | `Array#include?`/`member?`/`count` read one element, not a copy per element | +1.3% (layout) | O(n²) → O(n); no benchmark calls them |
| 2d | `17ab5dc` | `vm_optimization_bench` cut into five | — | the 50,000-entry Hash part is now `vmo_objects` |
| 6c | `6314b84` | a Ruby `method_missing` runs in the caller's frame (as the reference's) | ±1% (noise) | `call_args` +1.1%, `bm_fib` +0.7% over 15 rounds; the same change moved `bm_fib` −0.1% in another run |
| ECS | `ad54ed4` | `OP_GETIDX`/`GETIDX0`/`SETIDX` as the reference: fast paths for Array/Hash/String, else a send in the caller's frame | +0.1% | data structures −3.7% (`ds_hash` −8 to −9%, `vmo_index` −7%); `call_kwargs` +8% is code layout (candidates that fixed it cost +12–20% elsewhere) |
| 3 | `5c3eb6e` (merge of `93824b1`, `1472346`) | `Method::Closure`, host state | +1.8% | `bm_fib` +4.2%, `call_args` +4.0%, `app_tak` +3.8%: `Method`'s `Clone` is no longer a plain copy and `find_method` clones one per call (stage 2, candidate 3, removes that). `loop_while_add` +6.6% (reproduced twice, A/B on a quiet machine: 1100 → 1190 ms) is not explained by that — the loop makes no calls; `loop_times` moved −5.8% at the same time, so code layout is the likely cause. Data structures unchanged |

## Earlier measurements (the five reference benchmarks, best of 3)


mruby's `benchmark/*.rb`, compiled by the reference `mrbc`. Best of 3 runs. Reference `mruby` runs inside the
Docker image on the same machine (so the numbers are a ratio, not an absolute). Generated by `tools/bench.sh` on 2026-09-12.

| benchmark | mruby ms | SabiRuby ms | ratio | instructions | ns/instruction |
|---|---:|---:|---:|---:|---:|
| bm_ao_render | 2519 | fail: undefined method 'printf' for Object (NoMethodError) | | | |
| bm_fib | 1758 | 6223.078 | 3.53x | 742675842 | 8.4 |
| bm_mandel_term | 11 | fail: undefined method 'putc' for Object (NoMethodError) | | | |
| bm_so_lists | 242 | 3813.499 | 15.75x | 57014750 | 66.9 |
| bm_so_mandelbrot | 885 | 1707.520 | 1.92x | 341836153 | 5.0 |
| gc_churn | 92 | 360.368 | 3.91x | 15000650 | 24.0 |
| vm_optimization_bench | 3351 | 22853.327 | 6.81x | 1796631395 | 12.7 |

## Re-measured after the inspection hooks (2026-09-12)

`src/inspect.rs` added event recording to `frame_env`, `pop_frame`, `handle_raise`,
`unwind_return`, `switch_context` and `gc_collect`. The instruction loop (`exec_frames`) is
unchanged; with recording off (the default) the cost is one `Option` test at those places.
Same machine, same procedure, recording off:

| benchmark | before ms | after ms | change |
|---|---:|---:|---:|
| bm_fib | 6093.638 | 6165.897 | +1.2% |
| bm_so_lists | 3767.137 | 3781.102 | +0.4% |
| bm_so_mandelbrot | 1697.848 | 1611.633 | −5.1% |

All within the ±3% the plan asked for, apart from mandelbrot getting *faster*, which is the
run-to-run noise of this machine. See [`../design/inspect.md`](../design/inspect.md).

## Re-measured after the numeric tower, pack and eval (2026-09-12)

mruby-bigint, -rational, -complex, -cmath, -pack, -eval and -binding went in. The VM's fast
paths were not touched: an Integer operation still takes the `checked_*` route and only its
overflow exit reaches the wide-integer code, and `eval` is a native that calls the host.
`Class#new` changed from allocating directly to sending `allocate` (the reference's
`new_iseq` does), which is one method lookup per `new` — `bm_so_lists` (a list of 10000
objects rebuilt 16 times) shows no change.

| benchmark | before ms | after ms | change |
|---|---:|---:|---:|
| bm_fib | 6282.426 | 6223.078 | −0.9% |
| bm_so_lists | 3817.435 | 3813.499 | −0.1% |
| bm_so_mandelbrot | 1700.850 | 1707.520 | +0.4% |

`vm_optimization_bench` and `gc_churn` run for the first time here: the first needs `Time`
(mruby-time, ported 2026-09-12) and the second is new in the reference tree.

## Re-measured after UTF-8 strings (2026-09-12)

Strings became sequences of characters (the feature `utf8`, on by default; `docs/design/utf8.md`).
The character helpers take how the string is read as an argument, so every string method now
starts with one test of that flag, and the instruction loop is untouched. These three
benchmarks are numeric and list work with no string method in their inner loops, which is what
the numbers say; a string-heavy benchmark is not in mruby's set.

| benchmark | before ms | after ms | change |
|---|---:|---:|---:|
| bm_fib | 6223.078 | 6304.543 | +1.3% |
| bm_so_lists | 3813.499 | 3803.837 | −0.3% |
| bm_so_mandelbrot | 1707.520 | 1674.126 | −2.0% |

Within this machine's run-to-run spread (the same three moved ±1.2% between two runs of the
unchanged VM above).
