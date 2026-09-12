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

* **mruby-compar-ext**, **mruby-toplevel-ext**, **mruby-enum-lazy** — pure Ruby, loaded as is. The
  core stage had a native `clamp` on Comparable and on Integer; in the reference the only `clamp` is
  this gem's Ruby method, so both natives went (rule 4: the native took two arguments and shadowed the
  one-argument range form).
* **mruby-enum-chain** — pure Ruby. Its `rewind` test defines a singleton method on a Range literal,
  which uncovered a deviation: SabiRuby marked every Range **frozen** and used that flag as the
  reference's `RANGE_INITIALIZED` flag (`'initialize' called twice`). In the reference `(1..2).frozen?`
  is false, and since the singleton class of a frozen object is frozen too (`sc->frozen = o->frozen`),
  `define_method` on the range's singleton class raised FrozenError here. Now an uninitialised Range is
  a plain object until `initialize`/`initialize_copy` gives it its ends, and no Range is frozen.
  `Enumerator::Chain#size` with a Range fails on the reference as well (mruby-range-ext defines
  `Range#size`, the test expects nil).

* **mruby-object-ext** (`ext_object.rs`) — `NilClass#to_a/to_h/to_i/to_f`, `Kernel#itself`,
  `BasicObject#instance_exec`; `tap`, `then`/`yield_self` are its Ruby (the core-stage natives
  `tap`/`then` went). `instance_exec` on an Integer uncovered a rule of `mrb_singleton_class_ptr`:
  an Integer/Float/Symbol has no singleton class, the frame then has no target class of its own
  and `class B` inside the block goes to the block's lexical class (`Vm::call_block_with_self_kw`
  passes no override). Keywords given to `instance_exec` stay keywords for the block (the
  pending-kdict rule of `send`).
* **mruby-symbol-ext** (`ext_symbol.rs`) — `length`/`size` (bytes in this build), `slice`/`[]`,
  which hand the name to `String#slice` as the reference does; `Comparable`, `capitalize`,
  `casecmp`, `empty?`, `intern` are its Ruby (the core-stage native `empty?` went).
* **mruby-kernel-ext** (`ext_kernel.rs`) — `Integer()`/`Float()` are the reference's scanners
  ported byte for byte (`mrb_str_len_to_integer`, `mrb_str_len_to_dbl`, `mrb_read_float`; the
  float's value is parsed by `core` from the exact span, so rounding is right). `caller` is the
  reference's arithmetic over `Vm::backtrace`, new here: one `file:line:in method` entry per Ruby
  frame with debug info, the native itself first, located at the frame that called it (the
  reference locates a C frame at the nearest Ruby frame below it the same way). `__method__`
  reads the frame: its `mid`, else the environment's (a block answers the method it was written
  in). Two things came with it: **an alias is a proc of its own** (`ProcData::mid`, the
  reference's `MRB_PROC_ALIAS` with `body.mid`) so a frame of `alias m3 m1` has `mid` `:m1`,
  which `__method__`, `__callee__` and `super` see; and `raise`/`fail` is one function. The
  core-stage `Integer()`, `Float()`, `String()`, `Array()`, `__method__` moved here.
* **mruby-class-ext** (`ext_class.rs`) — `Module#<`, `<=`, `<=>`, `>`, `>=` with the reference's
  three answers (true, false, nil when unrelated; a TypeError for a non-module), `class_exec`/
  `module_exec` (keywords kept), `name` (frozen), `singleton_class?`; `Class#attached_object`,
  `subclasses` (a heap walk over `Heap::ids`). The core-stage `Module#<`, `<=`, `name` moved here.
* **mruby-numeric-ext** (`ext_numeric.rs`) — `remainder`, `pow(b, m)` with the reference's
  overflow rule, `digits`, `size`, `bit_length`, `odd?`, `even?`, `gcd`, `lcm`, `modulo`,
  `Integer.sqrt`, `Float#remainder`/`modulo`, `Float::EPSILON`.. constants. `zero?`, `nonzero?`,
  `positive?`, `negative?`, `integer?`, `allbits?`, `ceildiv` are its Ruby: the core-stage natives
  of those went, and `even?`, `odd?`, `size`, `bit_length`, `gcd` moved here. The core `Float#div`
  (`flo_idiv`) was missing and is in `numeric.rs` now.
* **mruby-objectspace** (`ext_objectspace.rs`) — `count_objects` (TOTAL = every slot, FREE = the
  swept ones, then the live `T_*` counts in the reference's type order), `each_object`. Its test
  counts hashes across a `GC.start`, which exposed **the stack root**: SabiRuby marked the whole
  register vector, so registers left over from returned frames kept garbage alive. Now the
  running frame's window is the limit (`base + nregs`, or the arguments still to be packed),
  as `mark_context_stack` marks `ci->stack + nregs`. The test's C helper `__gc_root_survivors`
  (`mrb_gc_register` counting) is in `mrbtest.rs`; it found that `Heap::is_free` said "live" for
  a slot the sweep had truncated away.
* **mruby-catch** (`ext_catch.rs`) — `catch` records its tag and the depth of the block's frame
  in `Vm::catch_tags`; `throw` finds the innermost entry with the same object and returns a
  `Break` to that frame, the mechanism a `return` from a nested block already uses, so `ensure`
  bodies run and `rescue Exception` does not see it (the reference raises an `RBreak` aimed at
  the frame of its bytecode `catch`). An unmatched tag raises the gem's `UncaughtThrowError`.

## Compiling the tests

`tools/mrbtest.sh` copies a gem's `test/<file>.rb` as `gem_<file>.rb`; a name an earlier gem
already used is qualified as `gem_<gem>_<file>.rb` (`numeric.rb` of string-ext and numeric-ext,
`range.rb` of range-ext and string-ext, whose second copy had silently replaced the first
until 2026-09-12).

`tools/mrbtest.sh` compiles the test files with `mrbc -g`, which keeps the LVAR section:
`local_variables` and `Proc#parameters` need the names, and the reference driver compiles the
tests from source so it has them too. Reading LVAR uncovered a bug of the loader: the symbol
count of the section is 32-bit (`write_lv_sym_table`), not 16-bit.


## Remaining gems (plan as of 2026-09-12)

The reference `mruby` command is built from `default.gembox` = stdlib, stdlib-ext,
stdlib-io, math, metaprog (33 gems). Ported: fiber, enumerator, array-ext,
enum-ext, hash-ext, range-ext, string-ext, sprintf, metaprog, proc-ext, method,
compar-ext, toplevel-ext, enum-chain, enum-lazy, object-ext, symbol-ext, kernel-ext,
class-ext, numeric-ext, catch, objectspace (22). Sizes are lines of the reference
C / mrblib Ruby / test.

| order | gem | C / Ruby / test | depends on | notes |
|---|---|---|---|---|
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
| 7 | mruby-task | 2390 / 46 / 860 | – | not in default.gembox but planned: `Task` (priority queues, `Task.pass`/`sleep`/`join`/`Task::Queue`, tick-based preemption) on top of the Fiber contexts and `Vm::step`; the HAL (timer tick, `sleep_us`, idle) comes from the host, like the compiler hook; the scheduler-driven GC of `docs/gc.md` is part of it. Its `mrb_task_run` blocks, so the host-loop form (`run_once`) is the one rubevy needs |

Order: 1 (pure Ruby, done 2026-09-12) → 2 (small natives, done 2026-09-12) → 3 (data structures and host
clocks) → 4 (eval, with the compiler hook) → 5 (numeric tower, pack) → UTF-8 strings
(`docs/utf8-plan.md`, a build-configuration milestone required for Japanese text) →
6 (regexp, on top of UTF-8) → 7 (task). regexp is by far the heaviest and can be moved after task.
Each gem: natives in `src/builtins/ext_<gem>.rs`, mrblib into `src/mrblib_<gem>.mrb`,
tests into `tools/mrbtest.sh` `GEMS`, reasons for what does not pass into
`docs/mrbtest-notes.md`, and the `Vm::with_mrblib` load order stays the gembox order.

### Core gems outside default.gembox

Candidates, with the reference sizes (C / Ruby / test):

| gem | size | worth it? |
|---|---|---|
| mruby-sleep | 186 / 0 / 29 | yes, with task: `Kernel#sleep`/`usleep` via the host clock hook |
| mruby-strftime | 118 / 0 / 152 | yes, with time: `Time#strftime` |
| mruby-string-bitops | 581 / 0 / 210 | maybe: `String#&`, `|`, `^`, `~` on bytes; small, self-contained |
| mruby-os-memsize | 283 / 0 / 63 | maybe: `ObjectSpace.memsize_of`; needs per-object sizes from our heap, answers will differ from the reference (deviation) |
| mruby-encoding | 109 / 0 / 921 | no for now: only meaningful with `MRB_UTF8_STRING`, which the byte-string build does not have |
| mruby-cmath | 425 / 0 / 41 | with complex (order 5) |
| mruby-benchmark | 0 / 130 / 283 | no: pure Ruby but depends on io and process |
| mruby-error, mruby-exit | 143, 82 | no: C API helpers (`mrb_protect`), `exit` is a host decision |
| mruby-test-inline-struct, mruby-test, mruby-bin-* | – | build/test infrastructure, not runtime |

Third-party gems are out of scope until the core list is done; the ones worth a look then
are pure-Ruby or small-C libraries used by PicoRuby (`picoruby-json`, `picoruby-yaml`,
`picoruby-base64`, `picoruby-crc`, `picoruby-markdown`) since their mrblib compiles as is.
