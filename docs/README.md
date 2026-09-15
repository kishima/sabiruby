# SabiRuby documents

Laid out as every repository of the organization is
([`.github/CONTRIBUTING.md`](https://github.com/sabiruby/.github/blob/main/CONTRIBUTING.md)):
`design/` says how it is built, `verification/` how it is checked and measured, `plans/` what
was decided to do and in what order, `worklog/` what happened when. Design and verification are
in English; plans and the worklog are in Japanese.

## design/ — how it is built

| file | what it covers |
|---|---|
| [compiler.md](design/compiler.md) | the reference compiler as a crate (`sabiruby-compiler`), the C shim, `eval`, the `Host` trait |
| [exceptions.md](design/exceptions.md) | unwinding with `Result` instead of `setjmp`/`longjmp`; `break`/`return` through natives |
| [fibers.md](design/fibers.md) | fibers without a second host stack; the native-boundary rule; fibers inside tasks |
| [gc.md](design/gc.md) | stop-the-world mark & sweep, roots, the contract for natives, the scheduler-driven mode, the free hook for `Data` |
| [gems.md](design/gems.md) | every ported gem, what deviates and why, mruby-task in depth (host entry points, time limits, how far the fork may drift) |
| [inspect.md](design/inspect.md) | snapshots, traces and line numbers: what a debugger or a HUD can read |
| [performance.md](design/performance.md) | the value representation, the `Slot` window, known structural costs (short) |
| [optimizations.md](design/optimizations.md) | (Japanese) the speed-ups of 2026-09-15 one by one — symptom, cause, change, effect, what was dropped — how to measure on this machine, what is still slow |
| [utf8.md](design/utf8.md) | strings as characters (feature `utf8`) and as bytes |
| [playground.md](design/playground.md) | the browser playground: the wasm module's C ABI, the debugger, real-time `sleep` |

## verification/ — how it is checked and measured

| file | what it covers |
|---|---|
| [mrbtest.md](verification/mrbtest.md) | mruby's own test suite on SabiRuby, per file (the default, character-string build) |
| [mrbtest-bytes.md](verification/mrbtest-bytes.md) | the same for the byte-string build |
| [mrbtest-notes.md](verification/mrbtest-notes.md) | why an assertion does not pass: every remaining failure with its reason |
| [bench.md](verification/bench.md) | benchmarks by category against the reference, the baseline and every stage since |
| [upstream-pr-candidates.md](verification/upstream-pr-candidates.md) | what the port found in mruby-task that belongs upstream |

## plans/ — what was decided, in order

| file | status |
|---|---|
| [compiler-plan.md](plans/compiler-plan.md) | done (2026-09-12): the compiler crate |
| [gc-plan.md](plans/gc-plan.md) | done (2026-09-11) |
| [gems-plan.md](plans/gems-plan.md) | done (2026-09-13): orders 4–7 of the gem work |
| [utf8-plan.md](plans/utf8-plan.md) | done (2026-09-13) |
| [eval-require-plan.md](plans/eval-require-plan.md) | done (2026-09-13); written as a study, kept for the reasoning |
| [after-gems-plan.md](plans/after-gems-plan.md) | done (2026-09-13): rubevy on one VM with tasks, backtraces, `sleep`/`strftime` |
| [playground-plan.md](plans/playground-plan.md) | done (2026-09-12); the playground repository carries the visualizer plan |
| [host-bridge-plan.md](plans/host-bridge-plan.md) | done through stage 5 and 2c (2026-09-15); stage 6 (macros, futures, proxies) is direction only. Carries the findings of each stage |

## worklog/ — what happened, when

One file per piece of work, `YYYY-MM-DD-<slug>.md`, written while the work happened. The four
of 2026-09-15 are the records of `host-bridge-plan.md`'s stages 2, 2b, 2c, 4, 5 and 3b
([stage3b-host-entry-points](worklog/2026-09-15-stage3b-host-entry-points.md): the entry points
that let rubevy stop reaching into `Vm`'s fields).

## Where things were (before 2026-09-15)

Every document above sat directly under `docs/`; references in older commits, in the book's notes
and in the worklog use those paths (`docs/gems.md` is now `docs/design/gems.md`, and so on).
