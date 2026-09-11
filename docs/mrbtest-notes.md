# Why an assertion does not pass

Hand-written companion of the generated [`mrbtest.md`](mrbtest.md). Every assertion of
mruby's `test/t` and of the ported gems that SabiRuby does not pass is listed here with
its reason, and with what the reference `mruby` (4.1.0-rc, default gembox, run through the
kit's `reference_runner.rb`) does on the same file. Update this file whenever the table
changes; `tests/mrbtest/notes.tsv` holds the one-line version that fills the `note` column.

Categories:

* **C fixture** — the test calls a native helper that only exists in mruby's `mrbtest`
  binary (`mrbgems/mruby-test/` or a gem's `test/*.c`). Porting the helper is possible but
  it tests mruby internals (REnv slots, `mrb_vformat`, `mrb_sys_fail`), not Ruby semantics.
  The reference `mruby` command fails these too.
* **GC** — SabiRuby has no collector yet; tests that count the GC arena cannot pass.
* **gem** — needs a gem that is not ported yet (`mruby-bigint`, `mruby-regexp`).
* **deviation** — a difference SabiRuby keeps on purpose (see README).
* **build** — depends on how the reference binary was built.

| file | not passing | category | reason |
|---|---|---|---|
| array | 1 crash | C fixture | `Array shared from an emptied heap array keeps a buffer` needs the `AryShared` class of `mruby-test/ary_shared.c`. The reference `mruby` crashes here too (62/63). |
| array | 1 skip | gem | `Array#sort with a block that answers with a big integer` skips itself: `requires mruby-bigint`. |
| env | 8 crash | C fixture | Every test calls `__env_svar?`, `__env_len`, `__env_cfunc_proc`, ... from `mruby-test/env.c`, probing REnv slot internals. The reference `mruby` crashes on all 8 too. |
| exception | 2 skip | build | `GC in rescue` and `Method call in rescue` skip when `backtrace_available?` is false. SabiRuby does not read the DBG section, so `Exception#backtrace` is empty. |
| float | 1 KO (3 assertions) | deviation | `a NaN is the object it is and no other`: SabiRuby Floats are immediates, so two NaNs made separately are `equal?` and have the same `object_id`. mruby with Word Boxing allocates each NaN on the heap. |
| gc | 5 KO | GC | `OP_GETIDX/GETIDX0/SETIDX does not retain ... in the GC arena`: they count arena entries after 20000 operations and expect `< 100`; SabiRuby has no arena (the count is the number of objects, 20001). |
| gc | 1 skip | gem | `OP_ADD does not retain an overflowed Integer`: `requires mruby-bigint`. |
| integer | 1 skip | gem | `Integer wider than an mrb_int compared with a NaN`: no bigint, so no such integer exists. |
| literals | 2 skip | build / gem | `Literals Numerical without Float` skips because Float is defined (the reference skips it too, 11/12); `Literals Numerical wider than mrb_int` skips without mruby-bigint. |
| superclass | 1 skip | gem | `Direct superclass of RegexpError`: `RegexpError` comes with mruby-regexp. |
| syntax | 1 KO (2 assertions) | deviation | `pattern matching - a key that moves the subject`: a hash pattern whose key's `hash`/`eql?` mutates the subject is not detected (mruby raises RuntimeError from the hash iteration guard). The reference `mruby` command has 3 KO here of its own: tests reading `__FILE__`/`__LINE__` see the concatenated runner script. |
| sysfail | 1 crash | C fixture | `TestSysFail` of `mruby-test/sysfail.c` (`mrb_sys_fail`). Reference `mruby` crashes too. |
| version | 1 skip | build | `MRUBY_REVISION` is `"HEAD"` in SabiRuby, and the test skips itself for a build without a revision. The reference binary carries the commit hash and passes. |
| vformat | 1 crash | C fixture | `TestVFormat` of `mruby-test/vformat.c` (`mrb_vformat`). Reference `mruby` crashes too. |
| regexperror | 0 assertions | gem | The file defines no assertion unless mruby-regexp is present; the reference reports 0 too. |
| gem_fiber2 | (all pass) | — | Needs the six natives of `mruby-fiber/test/fibertest.c`; SabiRuby provides them in `src/mrbtest.rs`. The reference `mruby` command lacks them and crashes on all 4. |

Summary (2026-09-11): 910 assertions, 882 pass. Not passing: 11 crashes (all C fixtures),
7 KO (5 GC arena, 2 deliberate deviations), 9 skips (bigint 3, regexp 1, backtrace 2,
Float defined 1, revision 1, plus the empty `regexperror`), 1 warning — see below.

## Warnings

`ensure - context - yield and return` reported "no assertion" until 2026-09-11: `return`
from a block inside a lambda, through a method with `ensure`, returned from the wrong frame
(the lambda's captured environment was used as the target instead of the environment of the
frame running the lambda). Fixed in `Vm::op_return_blk` following mruby's `top_proc`; the
file passes 5/5 now. If a warning reappears, the assertions inside the block did not run.
