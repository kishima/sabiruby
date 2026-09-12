# Browser playground

A static page where Ruby is written, compiled by the reference mruby compiler and run on the
SabiRuby VM, both as WebAssembly: **https://kishima.github.io/sabiruby-playground/**, source in
[kishima/sabiruby-playground](https://github.com/kishima/sabiruby-playground) (kept out of this
repository so that it stays small; Pages serves from there). The plan it follows is
[`playground-plan.md`](playground-plan.md); the departures are listed at the end.

## Shape

```
browser main thread                    Web Worker
  main.js  (editor, samples, output)     worker.js
  WebAssembly.compileStreaming  ──Module──▶ sabi.js ──C ABI──▶ sabiruby.wasm (wasm32-wasip1)
                                            browser_wasi_shim ◀── wasi_snapshot_preview1 imports
```

* `sabiruby.wasm` is the crate `sabiruby-wasm` (a cdylib, a WASI reactor) over `sabiruby` (the
  VM) and `sabiruby-compiler` (the reference compiler, C built with wasi-sdk). One VM per
  module instance. It imports only the WASI functions for stdio and the environment; the VM
  asks for no clock and no randomness (`hashbrown` with foldhash, fixed seed).
* The C ABI (no wasm-bindgen):

  | function | |
  |---|---|
  | `sabi_compile(src, len, debug) -> status` | Ruby source to RITE (`debug`: `mrbc -g`, keeps LVAR for `local_variables`); diagnostics in `sabi_take_text` |
  | `sabi_load(bin, len) -> status` | a RITE binary instead |
  | `sabi_reset() -> status` | a fresh VM with mrblib |
  | `sabi_start() -> status` | `Vm::load` + `Vm::start` of the compiled binary |
  | `sabi_step(budget: u32) -> 0 paused / 1 finished / 2 error` | `Vm::step`; the exception text in `sabi_take_text` |
  | `sabi_take_output(len_out) -> ptr` | `Vm::take_output` |
  | `sabi_take_text(len_out) -> ptr` | diagnostics (`FILE:LINE:COL: message`) or `describe_error` |
  | `sabi_dump(len_out) -> ptr` | `sabiruby::vm::dump` of the compiled binary |
  | `sabi_ast(src, len, len_out) -> ptr` | Prism's pretty-printed syntax tree (`sabiruby-compiler`, feature `ast`) |
  | `sabi_stats(insns, live, gc)` | three u64 out-parameters |
  | `sabi_trace(on)` | `Vm::set_trace`: record the events of `inspect.rs` |
  | `sabi_step_until(mode, budget) -> status` | 0 one instruction, 1 step over, 2 step into, 3 step out, 4 `budget` instructions (Continue, what Run uses). Step over keeps going while the frame is deeper than it was; step out until it is shallower. The loop is on the Rust side |
  | `sabi_step_program_only(on)` | stop only in the program's own ireps: mrblib and the gems are stepped through without stopping, where the listing has no row to show |
  | `sabi_state(regs_frames, len_out) -> ptr` | `Vm::snapshot` as JSON |
  | `sabi_take_trace(len_out) -> ptr` | the events since the last call, as JSON |
  | `sabi_gc_collect()` / `sabi_gc_stress(on)` | collect now; collect after every allocation |
  | `sabi_op_counts(len_out) -> ptr` | `[{"op":"MOVE","count":n}, ..]` |
  | `sabi_dump_json(len_out) -> ptr` | the listing as data: ireps, catch tables, and every instruction with its pc, line, opcode and operands |
  | `sabi_alloc` / `sabi_free`, `sabi_version` | buffers for JS; a NUL-terminated version string |

  Status: 0 ok, 1 compile error, 2 runtime error, 3 internal. Returned buffers stay valid until
  the next call that returns one.
* The page shows four panes in the order of the pipeline: code, AST (Prism's tree, as in the
  book's chapter 5), bytecode, result, plus a fifth, "VM インスペクタ", while debugging. AST and
  bytecode follow the editor (400 ms after the last change) and can be hidden.
* The worker runs `sabi_step(1_000_000)` in a loop and posts the output after each step.
  `Vm::step` pauses only at instruction boundaries of the top-level program and resumes where it
  stopped, fibers included (`fibers.md`), so the budget does not change the program's behaviour.
  **Stop** is `worker.terminate()`, which also stops a step in the middle; the page then starts a
  new worker from the same compiled `WebAssembly.Module` (no second download or compile).

## Debugging: the VM visualizer

The point of the playground is what the reference `mruby` on wasm cannot show: the VM between two
instructions. **デバッグ** compiles the program and stops before the first instruction; then the buttons of any
debugger: **ステップオーバー** (F10, the next line of this frame, calls running without stopping
inside them), **ステップイン** (F11), **ステップアウト** (Shift+F11), **命令ステップ** (Ctrl+F11,
one instruction), **続行** (F5), **再起動** (Ctrl+Shift+F5) and **停止** (Shift+F5). The keys are
the ones Visual Studio and VS Code use. The editor is read-only while a session runs.

* Worker messages, next to the existing `run`/`inspect`: `debug-start {src}` (reset, compile,
  start, recording on, returns the listing and the first state), `debug-step {mode, budget}`
  (`sabi_step_until`, returns the state, the events since the last step and the stats),
  `debug-gc {collect | stress}`, `debug-stop`, `op-counts`. **続行** streams output and progress
  the way Run does, so an endless loop still stops with the worker.
* The bytecode pane is built from `sabi_dump_json` as rows (`web/debug.js`): the running
  instruction gets `.current`, the frames below it `.caller`, and the pane scrolls to follow. The
  editor marks the current line (from the DBG section).
* **Inside mrblib the listing has nothing to show**, and that is most of the time: `3.times { }`
  is 50 instructions, 40 of them in `Integer#times`, which is Ruby in mrblib and was loaded before
  the program (its irep is below `offset`). Stepping there used to blank the highlight and look
  frozen, so now the call the VM is running keeps a dashed mark (`.calling`) and a banner above the
  listing names it with its irep, pc and line — numbers that change at every step. The
  **mrblib に入る** toggle turns it off: `sabi_step_program_only` then runs those ireps to their end
  instead of stopping inside them, and only the program's own instructions are stepped through.
* The "VM インスペクタ" pane has six tabs, all drawn from the snapshot and the trace
  ([`inspect.md`](inspect.md)): **コールスタック** (the frames and the registers of the selected
  one, named from `lv`), **スコープ（環境）** (the environments, attached or moved to the heap, and
  the `EnvCreate`/`EnvDetach` log), **例外** (the catch table lookups of the last raise, and
  break/return), **Fiber** (one card per context), **GC** (the heap counters, collect now, stress,
  and the history of collections), **命令プロファイル** (the histogram of executed opcodes).
* Hovering an opcode — in the listing or in the histogram — shows its definition, operand format
  and summary from `web/opcodes.json`, extracted by `tools/opcodes.sh` from the mruby porting
  kit's `dataset/opcodes.jsonl` (MIT, 119 records, 32 KB). This works outside a debug session too.
* Some samples carry a "見どころ" note (`web/samples/index.json`), shown as a toast when the
  session starts: `vm_closure.rb` for the environment tab, `cg_rescue.rb` for exceptions,
  `vm_fiber*.rb` for fibers, `gc_churn.rb` (this repository's own sample) for the GC tab.

The whole debugger adds 69,913 bytes to the module (16,580 gzipped): the JSON writer, the
snapshot and the DBG reader. The plan's budget was 100 KB.

## Building the compiler for wasm

`sabiruby-compiler`'s `build.rs` handles `wasm32-wasip1` when `CC_wasm32_wasip1` points at
wasi-sdk's clang (see `compiler.md`):

* `-std=gnu99` (as the reference build; strict `c99` hides `memccpy`, which wasi-libc then
  rejects as an implicit declaration).
* The compiler's `MRC_TRY`/`MRC_THROW` are `setjmp`/`longjmp`. wasi-libc implements them with
  **WebAssembly exception handling**: the C is compiled with `-mllvm -wasm-enable-sjlj
  -mllvm -wasm-use-legacy-eh=true`, and wasi-sdk's `libsetjmp.a` is linked, copied into
  `OUT_DIR` under its own name (putting wasi-sdk's lib directory on the search path made `-lc`
  pick wasi-sdk's libc against rustc's `crt1`: undefined `__wasi_init_tp`). Only the code
  generator's error path uses them; `alias :"a#{1}" :b` exercises it and gives the same
  diagnostics as natively. The legacy encoding runs on Chrome/Edge 95+, Firefox 100+,
  Safari 15.2+ and Node 22.
* The stack is set by the wasm crate (`-zstack-size`): 4 MB. The deepest cases, native re-entry
  until `SystemStackError` (`NATIVE_DEPTH_MAX`) and 250 nested literals (Prism's depth limit is
  256), already pass with the default 1 MB.

Module as deployed: 1,303,895 bytes after `wasm-opt -Oz`, 438,065 over gzip (Pages compresses it;
1,169,017 / 421,881 before the AST pane and the debugger).
Page ready (navigation start to the Run button enabled: fonts, CodeMirror, the module, the worker,
the VM with mrblib) on the deployed site in a fresh headless Chromium: 0.43–1.46 s over three runs
on 2026-09-12; about 0.37 s from a local server. Instantiating the module and creating the VM:
15 ms in Node.

## Size compared with other Ruby VMs on wasm (2026-09-12)

What a browser downloads (the gzip column; Pages and npm CDNs serve wasm compressed):

| module | contents | bytes | gzip -9 |
|---|---|---:|---:|
| `picoruby.wasm`, npm `@picoruby/wasm-wasi` 4.0.3 (as published) | mruby VM (C), compiler, many gems, JavaScript bridge | 2,104,568 | 871,069 |
| this playground's `sabiruby.wasm` (VM + compiler + AST + debugger, local build) | SabiRuby, reference compiler (C), `pm_prettyprint`, `inspect.rs` and the JSON writer | 1,303,895 | 438,065 |
| SabiRuby VM only (probe below) | `sabiruby` 0.2.0 incl. the embedded mrblib | 783,729 | 281,054 |
| mruby/edge VM only (probe below) | `mrubyedge` 1.1.12, default features (`wasi`, `mrubyedge-debug`) | 562,560 | 185,343 |
| `mrbc.wasm`, npm `@picoruby/mrbc` 4.0.3 (as published) | compiler only | 519,224 | 157,505 |

* The two VM-only rows are probes built the same way for `wasm32-unknown-unknown` (release,
  `lto = true`, `codegen-units = 1`, `panic = "abort"`, `strip = true`, then
  `wasm-opt -Oz`, binaryen 132): a cdylib exporting one function that loads a RITE binary from
  memory and runs it, so the whole VM is kept and nothing else is added:

  ```rust
  // mruby/edge
  let mut rite = mrubyedge::rite::load(data)?; mrubyedge::yamrb::vm::VM::open(&mut rite).run()?;
  // SabiRuby
  let mut vm = sabiruby::Vm::with_mrblib()?; vm.load_and_run(data)?;
  ```
* The compiler's share of the playground is about 140 KB gzipped (421 − 281), close to
  PicoRuby's stand-alone `mrbc.wasm` (158 KB): both are the Prism-based mruby compiler.
* mruby/edge is smaller mostly because it has no compiler (Ruby is compiled to bytecode ahead of
  time and embedded). VM against VM, SabiRuby is about 96 KB (gzip) larger; where that comes from
  (core library coverage, GC, fibers, …) was not measured. The embedded mrblib is 47,448 bytes
  (16,272 gzipped), not the main part.
* These compare download size only, not features or speed.

## Verification

Three suites in the playground repository, run by its CI before every deploy:

* `test/fixtures.mjs` (Node, the same `sabi.js` and browser_wasi_shim as the page): SabiRuby's 17
  fixtures compiled and run in wasm, stdout compared with the reference mruby's `.out`: all 17
  byte-identical.
* `test/api.mjs`: 23 checks of the paths the page depends on (compile and generator errors,
  uncaught exceptions, fibers across step budgets, an endless loop pausing, deep recursion, GC,
  dump, invalid UTF-8 output, and the debugger's: the shape of `sabi_state`, stepping by
  instruction / line / call, the trace of a closure and of a raise, `sabi_gc_collect`, the line
  numbers in `sabi_dump_json`, the opcode counts).
* `test/browser.mjs` (Playwright, headless Chromium): the page's own buttons; all 17 fixtures
  through **Compare with mruby**, Stop, the bytecode pane, share links, and the debugger
  (stepping moves the current instruction, the frame table, the opcode tooltip, a detached
  environment after running a closure, the GC and histogram tabs); no console errors.

The verification baseline of SabiRuby itself does not move: `tools/*.sh` and the golden tests
keep using the reference `mrbc` in Docker. The playground shows that the same VM and the same
compiler give the same results as wasm.

## Departures from the plan

* **Wasm exception handling is required** (the compiler's `setjmp`/`longjmp`), which the plan
  did not foresee; browsers without it (older than the versions above) cannot run the page.
* `-std=gnu99` in `sabiruby-compiler` for every target (the plan expected no change to build.rs
  beyond selecting the compiler).
* Stack 4 MB instead of 16 MB, by measurement.
* `wasm/` depends on `../sabiruby` by path, and CI checks out kishima/sabiruby at a pinned commit
  next to the playground, instead of a git dependency: one pin for the VM, the compiler and the
  fixtures.
* `sabi_step` takes a u32 budget (no BigInt in JS); `sabi_stats` also returns the number of
  collections.
* CodeMirror 6 is vendored as one esbuild bundle (the plan left cdnjs or vendoring open):
  offline use as a companion of the book, and exact versions.
* The Node test uses browser_wasi_shim rather than Node's `node:wasi`, so it exercises the page's
  import path.
* Samples: SabiRuby's own 17 fixtures (the porting kit's copy of `kwargs` differs), plus 45 of the
  book's example scripts (`overview_*`, `vm_*`, `corelib_*`, `gc_*`, `cg_*`).
* The bytecode pane shows `sabiruby dump`, modelled on `mrbc --verbose` but not identical
  (string literals appear as byte arrays); the page says so.

From `visualizer-plan.md` (the VM visualizer, 2026-09-12):

* The page's debugger lives in **`web/debug.js`**, not in `main.js` as the plan wrote: the panes
  and the listing are about 400 lines of drawing, and `main.js` keeps the editor, the worker and
  the samples. `main.js` owns the state and the messages.
* In the JSON a register carries its value **inline** (`{index, name, text, class, id}`) instead
  of a nested `value` object: it is the largest array in a snapshot, and the page reads it as one
  row. Environments keep the nested form.
* Trace events are tagged `kind` in `snake_case` (`env_create`, `catch_look`, …) rather than the
  Rust variant names.
* `GETUPVAR`/`SETUPVAR` show **how many levels** the instruction walks and which register it
  reaches, as a note on the environment tab; the plan asked for the walk itself, which the
  snapshot cannot show because `ProcView` carries only one `upper` link, not the chain.
* The bytecode pane is built from `sabi_dump_json` **always**, not only while debugging, so the
  opcode tooltip works everywhere; `inspect` therefore returns the structured listing next to the
  text one (its behaviour is unchanged otherwise).
* The buttons are a debugger's usual ones — step over, step into, step out, one instruction —
  rather than the plan's 1 命令 / 1 行 / 呼び出し／戻り (author's decision, 2026-09-12: "案 A で").
  The plan's "1 行" stopped on returns as well, and "呼び出し／戻り" has no counterpart in a
  common debugger; neither could skip over a call, which is the step people reach for most.
  `sabi_step_until` therefore compares the frame depth as well as the line, and F10/F11 now mean
  what they mean everywhere else (the plan had F10 on the step that enters calls).
* An extra worker message `op-counts` (the plan folded the histogram into the step reply, but it
  is only needed when the tab is open or a run has finished).
* `gc_churn.rb` is kept in this repository (`tools/samples/`) and copied by `tools/samples.sh`,
  because the porting kit has no such sample; the "見どころ" notes are a table in the same script,
  so `web/samples/index.json` stays generated.

## Limits

What the VM does not have yet (README "Not yet": bigint, `$~`/`$_`, the `io`/`time`/`math`
gems, encodings, `$!`) is missing in the playground too. The page also needs module workers (Firefox 114+). Output appears in chunks of one million
instructions, not line by line.
