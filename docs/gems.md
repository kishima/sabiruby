# Ported gems

How a gem of the reference tree becomes part of SabiRuby, and what each ported one taught.

## Recipe

1. **Natives**: the gem's `src/*.c` methods go into `src/builtins/ext_<gem>.rs` (or
   `fiber.rs`), registered after the core natives in `builtins::init` so they replace core
   natives of the same name, as the gem's `mrb_..._gem_init` does in the reference.
2. **Ruby part**: `tools/mrbtest.sh` concatenates the gem's `mrblib/*.rb`, compiles it with the
   reference `mrbc` into `src/mrblib_<gem>.mrb`, and `Vm::with_mrblib` loads it after the core
   mrblib **in the order of `mrbgems/default.gembox`** (`lib.rs`, `vm.rs`). The order matters
   when two gems define the same method: mruby-enumerator's `Enumerable#zip` replaces
   mruby-enum-ext's because it is initialised later.
3. **Tests**: add the gem to `GEMS` in `tools/mrbtest.sh`; its `test/*.rb` become
   `tests/mrbtest/gem_<file>.mrb` and run like the core files. C test helpers (`test/*.c`)
   are re-implemented in `src/mrbtest.rs`.
4. **Core natives that only exist because of a gem must go.** SabiRuby's core stage had
   `Array#zip`, `Kernel#to_enum` (a NotImplementedError stub), `Range#count`, `Hash#count`
   as natives; in the reference those are gem mrblib Ruby methods (or `Enumerable`'s). A
   native on the class shadows the Ruby definition inherited through `Enumerable`, so the
   gem's method never runs. Rule: before writing a native, check whether the reference
   defines the method in C at all.

## What each gem needed

* **mruby-fiber** — see `docs/fibers.md`.
* **mruby-enumerator** — pure Ruby; needed the `send`/`__send__` in-place dispatch,
  `initialize_copy` on `dup`/`clone`, and the removal of the `to_enum` stub.
* **mruby-array-ext** — 34 natives (`ext_array.rs`). The set operations (`-`, `|`, `&`,
  `difference`, `union`, `intersection`, `intersect?`, `__uniq`) compare with `eql?`, not
  `==` (the reference uses a khash set keyed by `hash`/`eql?`; here a linear walk with
  `eql?`). `__combination_init/next` and `__product_generate/next` keep their state in a
  plain Array instead of an RData. `insert` and `__fill_exec` refuse sizes above
  `ARY_MAX_SIZE` (2^28 here) with the reference's "array size too big".
* **mruby-enum-ext** — `__minmax` and `__count` on Array. `Kernel#<=>` (`mrb_obj_cmp`:
  0 for the same object or `==`, else nil) was missing and is needed by `min_by`/`max_by`.
  `OP_EQ` must answer true for the same object *before* sending `==` (`mrb_obj_eq`);
  SabiRuby only did that for immediates, which broke `count(obj)` on an object whose `==`
  is always false.
* **mruby-hash-ext** — `values_at`, `slice`, `slice!`, `except`, `key`, `__merge`,
  `Hash.[]`, plus the core `__compact` the gem's Ruby `compact!` calls.
* **mruby-range-ext** — `cover?` (range-in-range rules), `size` (Float-aware, with the
  reference's epsilon), `__empty_range?`. Its Ruby `first`/`last`/`min`/`max` replace the
  core natives. Note: with this gem loaded, `(1..).last` raises RangeError, and the core
  test `Range#last` (`assert_nil (1..).last`) fails on the reference `mruby` as well.
* **mruby-string-ext** — 56 natives (`ext_string.rs`) for the byte-string build. The
  `tr`/`delete`/`squeeze`/`count` pattern parser is ported byte for byte, including two
  quirks of the reference: the bitmap used by `delete`/`squeeze`/`count` treats a range
  `a-c` as *exclusive* of `c` (`tr_compile_pattern` loops `i < ch[1]`), while `tr` itself
  is inclusive; and replacement bytes are read as signed `char`, so a byte ≥ 0x80 in the
  replacement deletes. `String#succ` follows `str_succ_bang`: the rightmost alphanumeric
  steps and carries across non-alphanumerics, but a letter never carries into a digit nor a
  digit into a letter (`"1-z".succ == "1-aa"`, `"1.9".succ == "2.0"`). `Integer#chr` accepts
  only `ASCII-8BIT`/`BINARY` (a UTF-8 name is an ArgumentError in this build).

## Deviations kept

* NaN identity: every NaN is one immediate here, the reference allocates one object per NaN
  (`[nan].uniq`, `[nan].count(nan)`, `[nan] - [nan]`).
* `String#slice!`, `tr` and friends work on bytes; the multibyte tests skip themselves.
* **mruby-sprintf** — `Kernel#sprintf`/`format` (`ext_sprintf.rs`): the reference's state machine
  for flags, `n$`, `<name>`/`{name}` and `*`, with its error messages; integers follow
  `mrb_int_to_cstr`/`mrb_uint_to_cstr` including the `..f` two's-complement form for negative
  `%x`/`%o`/`%b`; floats are rendered with `core::fmt` (correct rounding) and reshaped to C's
  `%f`/`%e`/`%g` (exponent of at least two digits, `%g` trailing-zero removal, `#` keeps them).
  `String#%` is the gem's Ruby part. The core-stage `String#%` stub was removed.
* **mruby-metaprog** (`ext_metaprog.rs`) — instance/class variable name checks (`'@0' is not
  allowed as an instance variable name`), `methods`/`instance_methods` families with the
  `regular`/`inherit` argument and the reference's listing rule (first table wins, `undef`
  hides), `singleton_methods(recur)`, `local_variables` from the caller's irep names,
  `included_modules`, `constants(inherit)`, `Module.constants`, `Module.nesting`,
  `remove_method` (+ `method_removed` hook), `undefined_instance_methods`, `public_send` with
  `method_missing` fallback. Two structural fixes came with it: the natives SabiRuby's core stage
  had defined on **Object** are moved to **Kernel** at init (the reference defines them in
  `mrb_init_kernel`; owners, listings and `Kernel.instance_method(:inspect)` depend on it), and
  `Vm::funcall` now dispatches a user-defined `method_missing` like a SEND does.
* **mruby-proc-ext** (`ext_proc.rs`) — `Proc#inspect` (`#<Proc:0x... -:-> (lambda)`),
  `lambda?`, `parameters` (from the ENTER operand and the irep's local names),
  `source_location` (nil), `Kernel#proc`; `curry`, `<<`, `>>`, `===`, `yield` are Ruby.
  `proc { break }.call` raising LocalJumpError needed the orphan rule for natives: when a native
  returns, a non-strict block made by the calling frame and passed to it is marked orphan
  (`Vm::orphan_block_of_native`; the reference does it in `cipop` of the C frame).
* **mruby-method** (`ext_method.rs`) — `Method`/`UnboundMethod` as plain objects with the
  reference's ivars `_owner`, `_recv`, `_name`, `_proc`, `_klass` (plus `_missing` for
  `respond_to_missing?` methods). A native method has no Proc object here: it is looked up again
  by owner and name when called or compared (two natives are `==` when they are the same
  function). Native arity comes from a small table keyed by function (`Vm::native_arity`),
  `-1` otherwise, because SabiRuby natives carry no `MRB_ARGS_*` spec.

## Compiling the tests

`tools/mrbtest.sh` compiles the test files with `mrbc -g`, which keeps the LVAR section:
`local_variables` and `Proc#parameters` need the names, and the reference driver compiles the
tests from source so it has them too. Reading LVAR uncovered a bug of the loader: the symbol
count of the section is 32-bit (`write_lv_sym_table`), not 16-bit.


## Remaining gems (plan as of 2026-09-12)

The reference `mruby` command is built from `default.gembox` = stdlib, stdlib-ext,
stdlib-io, math, metaprog (33 gems). Ported: fiber, enumerator, array-ext,
enum-ext, hash-ext, range-ext, string-ext, sprintf, metaprog, proc-ext, method
(11). Sizes are lines of the reference C / mrblib Ruby / test.

| order | gem | C / Ruby / test | depends on | notes |
|---|---|---|---|---|
| 1 | mruby-compar-ext | 0 / 79 / 47 | – | pure Ruby (`clamp`) |
| 1 | mruby-toplevel-ext | 0 / 24 / 23 | – | pure Ruby (`include`/`private`/`public` at top level) |
| 1 | mruby-enum-chain | 0 / 149 / 108 | enumerator | pure Ruby |
| 1 | mruby-enum-lazy | 0 / 384 / 84 | enumerator, enum-ext | pure Ruby |
| 2 | mruby-object-ext | 127 / 33 / 83 | – | `instance_exec`, `Object#tap`, `NilClass#to_a` |
| 2 | mruby-symbol-ext | 111 / 72 / 101 | – | `Symbol#length`, `to_proc` is Ruby |
| 2 | mruby-kernel-ext | 333 / 0 / 151 | – | `Integer()`, `Float()`, `String()`, `Array()`, `Hash()`, `__method__`, `fail` |
| 2 | mruby-class-ext | 376 / 0 / 182 | – | `Class#subclasses`, `attached_object`, `Module#name` rules |
| 2 | mruby-numeric-ext | 538 / 125 / 137 | – | `Integer#chr`, `digits`, `pow(mod)`, `Float#nan?`; bigint's tests need it |
| 2 | mruby-catch | 148 / 29 / 86 | – | `catch`/`throw`: an `UncaughtThrowError` raised through the normal unwinding, no longjmp needed |
| 2 | mruby-objectspace | 187 / 0 / 71 | – | `ObjectSpace.count_objects`, `each_object`: needs the heap walk (GC exists now) |
| 3 | mruby-struct | 909 / 77 / 504 | – | `Struct` (used by mruby-process, mruby-data is its sibling) |
| 3 | mruby-data | 639 / 9 / 143 | – | `Data.define` |
| 3 | mruby-set | 1552 / 325 / 807 | enumerator, hash-ext | `Set` (Hash-backed) |
| 3 | mruby-random | 646 / 0 / 201 | – | xoshiro128; must reproduce the reference sequence for a given seed to pass the tests |
| 3 | mruby-math | 752 / 0 / 201 | – | `Math` via libm (already a dependency) |
| 3 | mruby-time | 1738 / 0 / 313 | – | `Time`: needs a clock from the host (no_std: a `Host` hook, like the compiler); `localtime` is POSIX, use UTC only and record the deviation |
| 4 | mruby-eval | 417 / 0 / 333 | binding, compiler | `docs/eval-require-plan.md`; `tests/custom` cases wait for it |
| 4 | mruby-binding | 523 / 0 / 102 | – (tests: proc-ext) | with eval |
| 4 | mruby-proc-binding | 75 / 0 / 22 | binding, proc-ext | `Proc#binding` |
| 5 | mruby-pack | 2133 / 0 / 278 | – | `Array#pack`/`String#unpack`; large but self-contained |
| 5 | mruby-bigint | 6409 / 0 / 529 | – (tests: numeric-ext) | `MRB_USE_BIGINT`: Integer overflow becomes bigint; changes core Integer semantics, decide together with the build configuration |
| 5 | mruby-rational | 1512 / 72 / 742 | – (tests: complex) | `Rational`; the lexer literals (`1r`) already compile |
| 5 | mruby-complex | 1087 / 295 / 325 | math | `Complex` |
| 5 | mruby-cmath | 425 / 0 / 41 | complex | not in default.gembox |
| 6 | mruby-regexp | 10940 / 42 / 10213 | enumerator, symbol-ext, string-ext | the NFA engine (4.0.0); the largest single piece, its own milestone |
| – | mruby-io, mruby-socket, mruby-errno, mruby-dir, mruby-env, mruby-signal, mruby-process | 3868+1429+334+530+223+108+1320 | POSIX | not planned: the VM is no_std; a host `Host` trait may offer `puts`-level output only. mruby-error and mruby-exit are C API helpers, not needed |
| – | mruby-task | 2390 / 46 / 860 | – | not a gem of default.gembox; the scheduler is planned separately (after Fiber context reuse) |

Order: 1 (pure Ruby, an afternoon) → 2 (small natives) → 3 (data structures and host
clocks) → 4 (eval, with the compiler hook) → 5 (numeric tower, pack) → 6 (regexp).
Each gem: natives in `src/builtins/ext_<gem>.rs`, mrblib into `src/mrblib_<gem>.mrb`,
tests into `tools/mrbtest.sh` `GEMS`, reasons for what does not pass into
`docs/mrbtest-notes.md`, and the `Vm::with_mrblib` load order stays the gembox order.
