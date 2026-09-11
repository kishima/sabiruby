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
  | `sabi_alloc` / `sabi_free`, `sabi_version` | buffers for JS; a NUL-terminated version string |

  Status: 0 ok, 1 compile error, 2 runtime error, 3 internal. Returned buffers stay valid until
  the next call that returns one.
* The page shows four panes in the order of the pipeline: code, AST (Prism's tree, as in the
  book's chapter 5), bytecode, result. AST and bytecode follow the editor (400 ms after the last
  change) and can be hidden.
* The worker runs `sabi_step(1_000_000)` in a loop and posts the output after each step.
  `Vm::step` pauses only at instruction boundaries of the top-level program and resumes where it
  stopped, fibers included (`fibers.md`), so the budget does not change the program's behaviour.
  **Stop** is `worker.terminate()`, which also stops a step in the middle; the page then starts a
  new worker from the same compiled `WebAssembly.Module` (no second download or compile).

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

Module as deployed: 1,169,017 bytes after `wasm-opt -Oz`, 421,881 over gzip (Pages compresses it).
Page ready (navigation start to the Run button enabled: fonts, CodeMirror, the module, the worker,
the VM with mrblib) on the deployed site in a fresh headless Chromium: 0.43–1.46 s over three runs
on 2026-09-12; about 0.37 s from a local server. Instantiating the module and creating the VM:
15 ms in Node.

## Size compared with other Ruby VMs on wasm (2026-09-12)

What a browser downloads (the gzip column; Pages and npm CDNs serve wasm compressed):

| module | contents | bytes | gzip -9 |
|---|---|---:|---:|
| `picoruby.wasm`, npm `@picoruby/wasm-wasi` 4.0.3 (as published) | mruby VM (C), compiler, many gems, JavaScript bridge | 2,104,568 | 871,069 |
| this playground's `sabiruby.wasm` (VM + compiler + AST, local build) | SabiRuby, reference compiler (C), `pm_prettyprint` | 1,233,982 | 421,485 |
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
* `test/api.mjs`: 14 checks of the paths the page depends on (compile and generator errors,
  uncaught exceptions, fibers across step budgets, an endless loop pausing, deep recursion, GC,
  dump, invalid UTF-8 output).
* `test/browser.mjs` (Playwright, headless Chromium): the page's own buttons; all 17 fixtures
  through **Compare with mruby**, Stop, the bytecode pane, share links, no console errors.

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

## Limits

What the VM does not have yet (README "Not yet": bigint, `$~`/`$_`, the `io`/`time`/`math`
gems, encodings, `$!`) is missing in the playground too. The page also needs module workers (Firefox 114+). Output appears in chunks of one million
instructions, not line by line.
