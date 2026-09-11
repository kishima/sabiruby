# Exceptions without setjmp/longjmp

mruby unwinds with `MRB_THROW` (longjmp) out of whatever C frame raised. SabiRuby has no
longjmp; the same job is done with `Result` and one explicit unwinding loop.

## Pieces

* `VmError::Raise(exc)` — a Ruby exception in flight. Every native and every VM helper
  returns `VmResult<T>`, so `?` propagates it to the nearest `run_loop`.
* `VmError::Break(obj)` — a `break` / non-local `return` in flight (mruby's RBreak object,
  tagged Break/Return/Jump/Stop). It travels the same way as an exception, which is exactly
  what mruby does via `MRB_THROW` with a Break instead of an exception.
* `VmError::Unimplemented`, `Internal`, `Rite` — converted to a Ruby `NotImplementedError`
  (Unimplemented) or surfaced to the embedder; never seen by Ruby code otherwise.
* `Vm::exc` — the current exception, read by `OP_EXCEPT`, same role as `mrb->exc`.
* `CallInfo` has no jmpbuf. The stack of frames (`Vm::ci`) is the only thing to unwind.

## The loop (`run_loop`)

`run_loop(stop_depth)` runs `exec_frames` until the frame at `stop_depth` returns.
`exec_frames` is the bytecode dispatch; it returns `Err(Raise)` the moment any opcode or
native fails. `run_loop` then calls `handle_raise`, which walks `ci` from the top down:

1. `catch_find(irep, pc)` — the innermost catch handler in the current irep's catch table
   whose range covers `pc` (mruby's `catch_handler_find`). If found: set `pc` to the handler,
   store the exception in `Vm::exc`, `continue` the loop. Rescue/ensure code then runs as
   ordinary bytecode (`OP_EXCEPT`, `OP_RESCUE`, `OP_RAISEIF`).
2. Otherwise `pop_frame()` (which also does mruby's `cipop` work: marking orphaned envs) and
   look at the next frame down.
3. When `i <= stop_depth` the exception leaves this `run_loop` as `Err(Raise)`.

So one `run_loop` invocation is one mruby "vm_exec with jmpbuf". The `stop_depth` frame is
the equivalent of the `mrb_vm_run` entry frame with `CINFO_SKIP`.

## Native → Ruby → raise

A native (say `Array#each`) calls `vm.funcall(...)` for the block. `funcall` pushes a frame
and starts a **nested** `run_loop(stop_depth = that frame)`. A raise inside the block is
handled by the nested loop; if no handler exists above `stop_depth` it comes back to the
native as `Err(Raise)`. The native propagates with `?`, which returns it from the opcode into
the outer `run_loop`, which continues the unwinding from the native's caller frame.

Nothing is skipped: mruby with longjmp jumps over the native's C frame; here the native's
Rust frame is unwound by an ordinary return. `ensure` in Ruby code between the two loops is
still honoured, because the outer loop reaches it through `catch_find` on the way down.
Rust-side cleanup in natives is RAII, so nothing needs a `finally`.

`Break` follows the same path (`resume_break` in the outer loop, mruby's `L_BREAK` /
`RBREAK_TAG_*` handling), which is how `break` out of a block passed to a native method
lands in the right frame.

## Limits (host stack)

The host stack is consumed only by native ↔ Ruby recursion (each `funcall` nests a
`run_loop`). Ruby ↔ Ruby calls do not recurse on the host stack (`CALL_LEVEL_MAX` = 512
frames gives `SystemStackError`). `native_depth` (`NATIVE_DEPTH_MAX` = 96) guards the native
recursion and raises `SystemStackError` instead of overflowing; the CLI also runs the VM on a
thread with a large stack.

## What this costs

Every native call site returns a `Result` and checks it; that is a branch per call, not a
jump table. Compared with setjmp per `mrb_protect`/`rescue` it is cheaper (no register save),
and it is `no_std`-friendly, which longjmp is not portable for.
