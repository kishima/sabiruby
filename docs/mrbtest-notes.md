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
* **gem** — needs a gem that is not ported yet (`mruby-regexp`).
* **deviation** — a difference SabiRuby keeps on purpose (see README).
* **build** — depends on how the reference binary was built.

| file | not passing | category | reason |
|---|---|---|---|
| array | 1 crash | C fixture | `Array shared from an emptied heap array keeps a buffer` needs the `AryShared` class of `mruby-test/ary_shared.c`. The reference `mruby` crashes here too (62/63). |
| env | 8 crash | C fixture | Every test calls `__env_svar?`, `__env_len`, `__env_cfunc_proc`, ... from `mruby-test/env.c`, probing REnv slot internals. The reference `mruby` crashes on all 8 too. |
| exception | 2 skip | build | `GC in rescue` and `Method call in rescue` skip when `backtrace_available?` is false. SabiRuby does not read the DBG section, so `Exception#backtrace` is empty. |
| float | 1 KO (3 assertions) | deviation | `a NaN is the object it is and no other`: SabiRuby Floats are immediates, so two NaNs made separately are `equal?` and have the same `object_id`. mruby with Word Boxing allocates each NaN on the heap. |
| literals | 1 skip | build | `Literals Numerical without Float` skips because Float is defined (the reference skips it too, 11/12). |
| superclass | 1 skip | gem | `Direct superclass of RegexpError`: `RegexpError` comes with mruby-regexp. |
| syntax | 1 KO (2 assertions) | deviation | `pattern matching - a key that moves the subject`: a hash pattern whose key's `hash`/`eql?` mutates the subject is not detected (mruby raises RuntimeError from the hash iteration guard). The reference `mruby` command has 3 KO here of its own: tests reading `__FILE__`/`__LINE__` see the concatenated runner script. |
| sysfail | 1 crash | C fixture | `TestSysFail` of `mruby-test/sysfail.c` (`mrb_sys_fail`). Reference `mruby` crashes too. |
| version | 1 skip | build | `MRUBY_REVISION` is `"HEAD"` in SabiRuby, and the test skips itself for a build without a revision. The reference binary carries the commit hash and passes. |
| vformat | 1 crash | C fixture | `TestVFormat` of `mruby-test/vformat.c` (`mrb_vformat`). Reference `mruby` crashes too. |
| regexperror | 0 assertions | gem | The file defines no assertion unless mruby-regexp is present; the reference reports 0 too. |
| range | 1 crash | reference too | `Range#last`: `assert_nil (1..).last` in the core test, but mruby-range-ext's Ruby `last` raises RangeError for an endless range. The reference `mruby` (default gembox) crashes on the same assertion (21/22); the two only agree in a build without the gem. |
| gem_array | 1 KO (4 assertions) | deviation | `Array#uniq, Array#- and Array#include? with a NaN`: the reference treats every NaN made as its own object (identity), SabiRuby's Floats are immediates. |
| gem_enum | 1 KO (2 assertions) | deviation | `Array#count with a NaN`: same NaN identity. |
| gem_enum_chain | 1 KO | reference too | `Enumerator::Chain#size`: `[1,2,3].chain(3..4).size` is 5 once mruby-range-ext gives `Range#size`, and the test expects nil. The reference `mruby` (default gembox) fails the same assertion (6/7). |
| gem_set | 1 KO | deviation | `Set#include? with an element that changes the Set`: the reference raises RuntimeError from the khash rebuild guard (GHSA-4jw6-mq65-g3c8); the Hash-shaped Set here has no such state and finishes the lookup. |
| gem_string | 9 skip | build | `swapcase`/`casecmp?` Unicode and the six `scrub` tests skip themselves without `MRB_UTF8_STRING` (`UNICODECASE` false); the reference build is a byte-string build too. |
| gem_fiber2 | (all pass) | — | Needs the six natives of `mruby-fiber/test/fibertest.c`; SabiRuby provides them in `src/mrbtest.rs`. The reference `mruby` command lacks them and crashes on all 4. |
| gem_array | (C helper) | — | `__unshift_from_c` of `mruby-array-ext/test/array.c` is provided by `src/mrbtest.rs`. |
| gem_sprintf | 3 skip | build | Tests of `%c` with UTF-8 code points and of the result's encoding skip unless `__ENCODING__ == "UTF-8"` / `String#encoding` exists; neither does on the reference build either. |
| gem_proc | 2 crash | C fixture | `ProcExtTest.mrb_proc_new_cfunc_with_env` / `mrb_cfunc_env_get` test the C closure API of `mruby-proc-ext/test/proc.c`; there is no C closure here. The reference `mruby` crashes too. |
| gem_proc | 1 skip | build | `Proc#source_location` skips when no debug info is available (DBG is not read). |
| gem_method | 2 skip | build | `Method#source_location` / `UnboundMethod#source_location`: same DBG reason. |

Summary (2026-09-12, after the numeric tower — bigint, rational, complex, cmath): 1778
assertions, 1738 pass.
Not passing: 14 crashes (13 C fixtures, 1 core-test-vs-gem conflict the reference shares), 6 KO
(deliberate deviations: NaN identity ×4, the pattern-matching guard, the Set rebuild guard),
20 skips (regexp 1, backtrace 2, Float defined 1, revision 1, UTF-8/encoding 12, DBG-dependent
`source_location` 3, plus the empty `regexperror`), 0 warnings (one was a bug, see below).
The four assertions that skipped for want of mruby-bigint (`array`, `gc`, `integer`,
`literals`) run now; the four gems' own test files add 244 (bigint 29, rational 134,
complex 8 + 81, cmath 21), all passing.

The five `... does not retain ... in the GC arena` assertions of `gc` (`OP_GETIDX`, `OP_GETIDX0` twice each,
`OP_SETIDX`) failed until the collector (`docs/gc.md`): they compare `GC.stat[:live]` around 20000 operations after a
`GC.start` and expect a rise under 100. SabiRuby has no arena; the objects are simply collected, so they pass.

## Warnings

`ensure - context - yield and return` reported "no assertion" until 2026-09-11: `return`
from a block inside a lambda, through a method with `ensure`, returned from the wrong frame
(the lambda's captured environment was used as the target instead of the environment of the
frame running the lambda). Fixed in `Vm::op_return_blk` following mruby's `top_proc`; the
file passes 5/5 now. If a warning reappears, the assertions inside the block did not run.
